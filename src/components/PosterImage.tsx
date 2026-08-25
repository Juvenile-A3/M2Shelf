import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";
import { posterRenderLayout, shouldContainPosterArtwork, type PosterRenderLayout } from "../lib/poster";

interface PosterImageProps {
  active?: boolean;
  alt: string;
  cacheKey: string;
  onError: () => void;
  src: string;
}

const MAX_DEVICE_PIXEL_RATIO = 2;
const MAX_CONCURRENT_POSTER_RENDERS = 2;
const MAX_DOWNSCALE_RATIO_PER_PASS = 2;
const MAX_CACHED_POSTER_BITMAPS = 128;
const MAX_CACHED_POSTER_BITMAP_BYTES = 128 * 1024 * 1024;

interface CachedPosterBitmap {
  bitmap: ImageBitmap;
  bytes: number;
  canvasHeight: number;
  canvasWidth: number;
  contain: boolean;
  destinationX: number;
  destinationY: number;
}

const cachedPosterBitmaps = new Map<string, CachedPosterBitmap>();
let cachedPosterBitmapBytes = 0;

function posterBitmapCacheKey(cacheKey: string, width: number, height: number) {
  return `${cacheKey}:${width}x${height}`;
}

function removeCachedPosterBitmap(key: string) {
  const existing = cachedPosterBitmaps.get(key);
  if (!existing) return;
  cachedPosterBitmaps.delete(key);
  cachedPosterBitmapBytes -= existing.bytes;
  existing.bitmap.close();
}

function getCachedPosterBitmap(key: string) {
  const cached = cachedPosterBitmaps.get(key);
  if (!cached) return null;
  cachedPosterBitmaps.delete(key);
  cachedPosterBitmaps.set(key, cached);
  return cached;
}

function rememberCachedPosterBitmap(key: string, cached: CachedPosterBitmap) {
  removeCachedPosterBitmap(key);
  cachedPosterBitmaps.set(key, cached);
  cachedPosterBitmapBytes += cached.bytes;
  while (cachedPosterBitmaps.size > MAX_CACHED_POSTER_BITMAPS
    || cachedPosterBitmapBytes > MAX_CACHED_POSTER_BITMAP_BYTES) {
    const oldestKey = cachedPosterBitmaps.keys().next().value;
    if (oldestKey == null) break;
    removeCachedPosterBitmap(oldestKey);
  }
}

function drawCachedPosterBitmap(canvas: HTMLCanvasElement, cached: CachedPosterBitmap) {
  try {
    if (canvas.width !== cached.canvasWidth) canvas.width = cached.canvasWidth;
    if (canvas.height !== cached.canvasHeight) canvas.height = cached.canvasHeight;
    const context = canvas.getContext("2d", { alpha: true });
    if (!context) return false;
    context.clearRect(0, 0, cached.canvasWidth, cached.canvasHeight);
    context.imageSmoothingEnabled = true;
    context.imageSmoothingQuality = "high";
    context.drawImage(cached.bitmap, cached.destinationX, cached.destinationY);
    return true;
  } catch {
    return false;
  }
}

function isInsideVisibleScrollport(canvas: HTMLCanvasElement) {
  const rect = canvas.getBoundingClientRect();
  const scrollRoot = canvas.closest(".content-scroll");
  const rootRect = scrollRoot?.getBoundingClientRect() ?? {
    bottom: window.innerHeight,
    left: 0,
    right: window.innerWidth,
    top: 0,
  };
  return rect.width > 0
    && rect.height > 0
    && rect.bottom > rootRect.top
    && rect.top < rootRect.bottom
    && rect.right > rootRect.left
    && rect.left < rootRect.right;
}

interface PosterRenderTask {
  cancel: () => void;
  promise: Promise<void>;
}

interface QueuedPosterRender {
  cancel: () => void;
  start: () => void;
}

const posterRenderQueue: QueuedPosterRender[] = [];
let activePosterRenders = 0;

function schedulePosterRender(render: () => Promise<void>): PosterRenderTask {
  let started = false;
  let settled = false;
  let resolveTask: () => void = () => undefined;
  const promise = new Promise<void>((resolve) => { resolveTask = resolve; });
  const queued: QueuedPosterRender = {
    start: () => {
      if (settled) return;
      started = true;
      activePosterRenders += 1;
      void render()
        .catch(() => undefined)
        .finally(() => {
          activePosterRenders -= 1;
          posterRenderQueue.shift()?.start();
          settled = true;
          resolveTask();
        });
    },
    cancel: () => {
      if (started || settled) return;
      settled = true;
      const index = posterRenderQueue.indexOf(queued);
      if (index >= 0) posterRenderQueue.splice(index, 1);
      resolveTask();
    },
  };
  if (activePosterRenders < MAX_CONCURRENT_POSTER_RENDERS) queued.start();
  else posterRenderQueue.push(queued);
  return { cancel: queued.cancel, promise };
}

/**
 * Chromium's nominal "high" resize can still alias thin, high-contrast artwork when one pass
 * reduces a large provider image by several times. Keep every resize step at two-to-one or less,
 * then draw the final device-pixel-sized bitmap without another scaling pass.
 */
async function createProgressivelyDownscaledBitmap(
  image: HTMLImageElement,
  layout: PosterRenderLayout,
  shouldCancel: () => boolean,
): Promise<ImageBitmap | null> {
  const sourceX = Math.max(0, Math.floor(layout.sourceX));
  const sourceY = Math.max(0, Math.floor(layout.sourceY));
  const sourceRight = Math.min(image.naturalWidth, Math.ceil(layout.sourceX + layout.sourceWidth));
  const sourceBottom = Math.min(image.naturalHeight, Math.ceil(layout.sourceY + layout.sourceHeight));
  const sourceWidth = Math.max(1, sourceRight - sourceX);
  const sourceHeight = Math.max(1, sourceBottom - sourceY);
  const destinationWidth = layout.destinationWidth;
  const destinationHeight = layout.destinationHeight;

  const nextDimension = (current: number, destination: number) => (
    current <= destination
      ? destination
      : Math.max(destination, Math.ceil(current / MAX_DOWNSCALE_RATIO_PER_PASS))
  );
  const cancelled = () => {
    if (!shouldCancel()) return false;
    return true;
  };
  if (cancelled()) return null;

  // Crop and perform the first bounded reduction in one operation. Building a full-resolution
  // cropped bitmap first briefly doubled memory for large provider artwork and blocked the queue.
  const firstWidth = nextDimension(sourceWidth, destinationWidth);
  const firstHeight = nextDimension(sourceHeight, destinationHeight);
  let current = await createImageBitmap(image, sourceX, sourceY, sourceWidth, sourceHeight, {
    resizeWidth: firstWidth,
    resizeHeight: firstHeight,
    resizeQuality: "high",
  });
  if (cancelled()) {
    current.close();
    return null;
  }
  let returnCurrent = false;
  try {
    while (current.width !== destinationWidth || current.height !== destinationHeight) {
      const nextWidth = nextDimension(current.width, destinationWidth);
      const nextHeight = nextDimension(current.height, destinationHeight);
      if (cancelled()) return null;
      const next: ImageBitmap = await createImageBitmap(current, {
        resizeWidth: nextWidth,
        resizeHeight: nextHeight,
        resizeQuality: "high",
      });
      if (cancelled()) {
        next.close();
        return null;
      }
      current.close();
      current = next;
    }
    returnCurrent = true;
    return current;
  } finally {
    if (!returnCurrent) current.close();
  }
}

/**
 * WebView2 currently downsamples large, high-contrast poster art rather coarsely when an <img> is
 * painted inside the clipped card surface. Render once at the frame's physical pixel dimensions
 * instead: createImageBitmap performs the high-quality resize and the visible canvas is then a
 * one-device-pixel bitmap. The source data and cached cover remain untouched.
 */
export function PosterImage({ active = true, alt, cacheKey, onError, src }: PosterImageProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const errorHandlerRef = useRef(onError);
  const renderGenerationRef = useRef(0);
  const [wideArtwork, setWideArtwork] = useState(false);
  const [canvasReady, setCanvasReady] = useState(false);

  useEffect(() => { errorHandlerRef.current = onError; }, [onError]);
  useLayoutEffect(() => () => {
    // Invalidate asynchronous decode/resample work during the unmount commit, before passive
    // effect cleanup runs and before a late promise can touch the detached Canvas.
    renderGenerationRef.current += 1;
  }, []);
  useLayoutEffect(() => {
    renderGenerationRef.current += 1;
    const canvas = canvasRef.current;
    if (!canvas || (!active && !isInsideVisibleScrollport(canvas))) {
      if (canvas) {
        canvas.width = 1;
        canvas.height = 1;
      }
      setCanvasReady(false);
      return;
    }
    const rect = canvas.getBoundingClientRect();
    const pixelRatio = Math.min(MAX_DEVICE_PIXEL_RATIO, Math.max(1, window.devicePixelRatio || 1));
    const targetWidth = Math.max(1, Math.round(rect.width * pixelRatio));
    const targetHeight = Math.max(1, Math.round(rect.height * pixelRatio));
    const bitmapKey = posterBitmapCacheKey(cacheKey, targetWidth, targetHeight);
    const cached = getCachedPosterBitmap(bitmapKey);
    if (cached && drawCachedPosterBitmap(canvas, cached)) {
      setWideArtwork(cached.contain);
      setCanvasReady(true);
      return;
    }
    if (cached) removeCachedPosterBitmap(bitmapKey);
    canvas.width = 1;
    canvas.height = 1;
    setCanvasReady(false);
    setWideArtwork(false);
  }, [active, cacheKey, src]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    if (!active) {
      // Only the bounded high-DPR working-set LRU makes an already loaded poster inactive. Keep
      // the lightweight native preview mounted while releasing this large Canvas backing store.
      // A freshly remounted, actually visible card may already have synchronously restored its
      // cached bitmap in the layout phase; preserve it until IntersectionObserver activates it.
      if (!isInsideVisibleScrollport(canvas)) {
        canvas.width = 1;
        canvas.height = 1;
        setCanvasReady(false);
      }
      return;
    }

    const renderGeneration = renderGenerationRef.current;
    let disposed = false;
    let rendered = false;
    let renderScheduled = false;
    let rerenderRequested = false;
    let animationFrame = 0;
    let activeImage: HTMLImageElement | null = null;
    let scheduledRenderTask: PosterRenderTask | null = null;
    let observedDeviceWidth = 0;
    let observedDeviceHeight = 0;
    let resolutionQuery: MediaQueryList | null = null;
    const isStaleRender = () => disposed || renderGenerationRef.current !== renderGeneration;
    const render = async () => {
      if (isStaleRender()) return;
      const rect = canvas.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      const pixelRatio = Math.min(MAX_DEVICE_PIXEL_RATIO, Math.max(1, window.devicePixelRatio || 1));
      // Chromium reports the exact painted size in physical pixels. Matching that size avoids a
      // second compositor resize at Windows' fractional 125/150/175% display scales.
      const targetWidth = observedDeviceWidth || Math.max(1, Math.round(rect.width * pixelRatio));
      const targetHeight = observedDeviceHeight || Math.max(1, Math.round(rect.height * pixelRatio));
      if (rendered && canvas.width === targetWidth && canvas.height === targetHeight) return;
      const bitmapKey = posterBitmapCacheKey(cacheKey, targetWidth, targetHeight);
      const cached = getCachedPosterBitmap(bitmapKey);
      if (cached) {
        if (isStaleRender()) return;
        if (drawCachedPosterBitmap(canvas, cached)) {
          rendered = true;
          setWideArtwork(cached.contain);
          setCanvasReady(true);
          return;
        }
        removeCachedPosterBitmap(bitmapKey);
      }
      if (canvas.width !== targetWidth || canvas.height !== targetHeight) setCanvasReady(false);

      const image = new Image();
      activeImage = image;
      image.decoding = "async";
      image.src = src;
      const releaseImage = () => {
        if (activeImage === image) activeImage = null;
        image.removeAttribute("src");
      };
      try {
        await image.decode();
      } catch {
        releaseImage();
        return;
      }
      if (isStaleRender() || activeImage !== image) {
        releaseImage();
        return;
      }

      const contain = shouldContainPosterArtwork(image.naturalWidth, image.naturalHeight);
      const layout = posterRenderLayout(image.naturalWidth, image.naturalHeight, targetWidth, targetHeight, contain);
      if (!layout || isStaleRender()) {
        releaseImage();
        return;
      }
      setWideArtwork(contain);

      if (canvas.width !== targetWidth) canvas.width = targetWidth;
      if (canvas.height !== targetHeight) canvas.height = targetHeight;
      const context = canvas.getContext("2d", { alpha: true });
      if (!context) {
        releaseImage();
        return;
      }
      context.clearRect(0, 0, targetWidth, targetHeight);
      context.imageSmoothingEnabled = true;
      context.imageSmoothingQuality = "high";

      let bitmap: ImageBitmap | null = null;
      let drawSucceeded = false;
      let bitmapRetained = false;
      try {
        if (typeof createImageBitmap === "function") {
          bitmap = await createProgressivelyDownscaledBitmap(
            image,
            layout,
            () => isStaleRender() || activeImage !== image,
          );
        }
        if (isStaleRender() || activeImage !== image) return;
        if (bitmap) {
          context.drawImage(bitmap, layout.destinationX, layout.destinationY);
        } else {
          context.drawImage(
            image,
            layout.sourceX,
            layout.sourceY,
            layout.sourceWidth,
            layout.sourceHeight,
            layout.destinationX,
            layout.destinationY,
            layout.destinationWidth,
            layout.destinationHeight,
          );
        }
        drawSucceeded = true;
        if (bitmap && !isStaleRender()) {
          rememberCachedPosterBitmap(bitmapKey, {
            bitmap,
            bytes: bitmap.width * bitmap.height * 4,
            canvasHeight: targetHeight,
            canvasWidth: targetWidth,
            contain,
            destinationX: layout.destinationX,
            destinationY: layout.destinationY,
          });
          bitmapRetained = true;
        }
      } catch {
        if (!isStaleRender() && activeImage === image) {
          try {
            context.drawImage(
              image,
              layout.sourceX,
              layout.sourceY,
              layout.sourceWidth,
              layout.sourceHeight,
              layout.destinationX,
              layout.destinationY,
              layout.destinationWidth,
              layout.destinationHeight,
            );
            drawSucceeded = true;
          } catch { /* Keep the native preview visible when only Canvas rendering fails. */ }
        }
      } finally {
        if (!bitmapRetained) bitmap?.close();
        releaseImage();
      }
      rendered = drawSucceeded;
      if (drawSucceeded && !isStaleRender()) setCanvasReady(true);
    };

    const scheduleRender = () => {
      if (isStaleRender()) return;
      cancelAnimationFrame(animationFrame);
      animationFrame = requestAnimationFrame(() => {
        if (renderScheduled) {
          rerenderRequested = true;
          return;
        }
        renderScheduled = true;
        const task = schedulePosterRender(render);
        scheduledRenderTask = task;
        void task.promise.finally(() => {
          if (scheduledRenderTask === task) scheduledRenderTask = null;
          renderScheduled = false;
          if (rerenderRequested) {
            rerenderRequested = false;
            scheduleRender();
          }
        });
      });
    };
    const resizeObserver = typeof ResizeObserver === "undefined" ? null : new ResizeObserver((entries) => {
      const deviceSize = entries[0]?.devicePixelContentBoxSize?.[0];
      const rect = canvas.getBoundingClientRect();
      observedDeviceWidth = deviceSize
        ? Math.max(1, Math.min(Math.round(deviceSize.inlineSize), Math.round(rect.width * MAX_DEVICE_PIXEL_RATIO)))
        : 0;
      observedDeviceHeight = deviceSize
        ? Math.max(1, Math.min(Math.round(deviceSize.blockSize), Math.round(rect.height * MAX_DEVICE_PIXEL_RATIO)))
        : 0;
      scheduleRender();
    });
    if (resizeObserver) {
      try {
        resizeObserver.observe(canvas, { box: "device-pixel-content-box" });
      } catch {
        resizeObserver.observe(canvas);
      }
    }

    const bindResolutionListener = () => {
      resolutionQuery?.removeEventListener("change", handleResolutionChange);
      resolutionQuery = typeof window.matchMedia === "function"
        ? window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`)
        : null;
      resolutionQuery?.addEventListener("change", handleResolutionChange);
    };
    function handleResolutionChange() {
      // Per-monitor DPI can change while the CSS box stays the same, so the ResizeObserver alone
      // is insufficient. Discard the old physical dimensions and render for the new scale now.
      observedDeviceWidth = 0;
      observedDeviceHeight = 0;
      bindResolutionListener();
      scheduleRender();
    }
    bindResolutionListener();
    window.addEventListener("resize", scheduleRender);
    scheduleRender();

    return () => {
      disposed = true;
      cancelAnimationFrame(animationFrame);
      scheduledRenderTask?.cancel();
      resizeObserver?.disconnect();
      resolutionQuery?.removeEventListener("change", handleResolutionChange);
      window.removeEventListener("resize", scheduleRender);
      if (activeImage) {
        activeImage.removeAttribute("src");
        activeImage = null;
      }
    };
  }, [active, cacheKey, src]);

  return (
    <Fragment>
      {!canvasReady && (
        <img
          key={cacheKey}
          alt=""
          aria-hidden="true"
          className={`poster-image poster-image-preview${wideArtwork ? " is-wide-artwork" : ""}`}
          decoding="async"
          loading={active ? "eager" : "lazy"}
          onError={() => errorHandlerRef.current()}
          onLoad={(event) => setWideArtwork(shouldContainPosterArtwork(
            event.currentTarget.naturalWidth,
            event.currentTarget.naturalHeight,
          ))}
          src={src}
        />
      )}
      <canvas
        aria-label={alt}
        className={`poster-image poster-image-canvas${wideArtwork ? " is-wide-artwork" : ""}`}
        ref={canvasRef}
        role="img"
      >
        {alt}
      </canvas>
    </Fragment>
  );
}

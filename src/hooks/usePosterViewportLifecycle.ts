import { useEffect, useRef, useState, type RefObject } from "react";

interface PosterViewportLifecycleOptions {
  activationMarginPx: number;
  retentionEnabled?: boolean;
  retentionMarginPx: number;
}

interface PosterViewportLifecycle {
  coverRequested: boolean;
  coverVisible: boolean;
}

interface RetainedPoster {
  nearViewport: boolean;
  release: () => void;
}

const MAX_RETAINED_POSTERS = 64;
const retainedPosters = new Map<symbol, RetainedPoster>();

function trimRetainedPosters() {
  while (retainedPosters.size > MAX_RETAINED_POSTERS) {
    const oldestOffscreen = [...retainedPosters.entries()]
      .find(([, poster]) => !poster.nearViewport);
    if (!oldestOffscreen) return;
    const [token, poster] = oldestOffscreen;
    retainedPosters.delete(token);
    poster.release();
  }
}

function retainPoster(token: symbol, poster: RetainedPoster) {
  retainedPosters.delete(token);
  retainedPosters.set(token, poster);
  trimRetainedPosters();
}

function updateRetainedPosterProximity(token: symbol, nearViewport: boolean) {
  const poster = retainedPosters.get(token);
  if (!poster) return;
  poster.nearViewport = nearViewport;
  if (nearViewport) {
    retainedPosters.delete(token);
    retainedPosters.set(token, poster);
  }
  trimRetainedPosters();
}

function forgetRetainedPoster(token: symbol) {
  retainedPosters.delete(token);
}

/**
 * Preheat poster data before it reaches the visible scrollport, then retain the rendered
 * canvas across fast scroll reversals. Loaded posters are not released merely because they
 * leave the viewport or because a timer elapsed. A shared 64-entry LRU is the memory boundary:
 * once full, it evicts only the oldest poster outside the retention zone.
 */
export function usePosterViewportLifecycle<T extends Element>(
  targetRef: RefObject<T | null>,
  identity: string | number,
  {
    activationMarginPx,
    retentionEnabled = true,
    retentionMarginPx,
  }: PosterViewportLifecycleOptions,
): PosterViewportLifecycle {
  const retentionTokenRef = useRef(Symbol("retained-poster"));
  const [lifecycle, setLifecycle] = useState<PosterViewportLifecycle>({
    coverRequested: false,
    coverVisible: false,
  });

  useEffect(() => {
    const target = targetRef.current;
    let disposed = false;
    const retentionToken = retentionTokenRef.current;
    setLifecycle({ coverRequested: false, coverVisible: false });

    const releaseRetainedPoster = () => {
      if (!disposed) setLifecycle({ coverRequested: false, coverVisible: false });
    };
    const activate = () => {
      if (disposed) return;
      if (retentionEnabled) {
        retainPoster(retentionToken, {
          nearViewport: true,
          release: releaseRetainedPoster,
        });
      }
      setLifecycle((current) => (
        current.coverRequested && current.coverVisible
          ? current
          : { coverRequested: true, coverVisible: true }
      ));
    };

    if (!target) {
      activate();
      return () => {
        disposed = true;
        forgetRetainedPoster(retentionToken);
      };
    }
    if (typeof IntersectionObserver === "undefined") {
      if (retentionEnabled) {
        retainPoster(retentionToken, {
          // Without geometry support the hard cap is safer than an unbounded soft overage.
          nearViewport: false,
          release: releaseRetainedPoster,
        });
      }
      setLifecycle({ coverRequested: true, coverVisible: true });
      return () => {
        disposed = true;
        forgetRetainedPoster(retentionToken);
      };
    }

    // The application scrolls inside this element rather than the browser window. Using it as
    // the explicit root makes rootMargin a real preheat distance instead of having it clipped
    // away by the overflow ancestor.
    const scrollRoot = target.closest(".content-scroll");
    const activationObserver = new IntersectionObserver((entries) => {
      if (disposed) return;
      if (entries.some((entry) => entry.target === target && entry.isIntersecting)) activate();
    }, {
      root: scrollRoot,
      rootMargin: `${activationMarginPx}px 0px`,
    });
    const retentionObserver = new IntersectionObserver((entries) => {
      if (disposed) return;
      const nearViewport = entries.some((entry) => entry.target === target && entry.isIntersecting);
      if (retentionEnabled) updateRetainedPosterProximity(retentionToken, nearViewport);
    }, {
      root: scrollRoot,
      rootMargin: `${retentionMarginPx}px 0px`,
    });
    activationObserver.observe(target);
    retentionObserver.observe(target);

    return () => {
      disposed = true;
      forgetRetainedPoster(retentionToken);
      activationObserver.disconnect();
      retentionObserver.disconnect();
    };
  }, [activationMarginPx, identity, retentionEnabled, retentionMarginPx, targetRef]);

  return lifecycle;
}

// Bangumi occasionally exposes a landscape or near-square subject image instead of poster art.
// Cropping that image into a tall frame makes WebView upscale its short edge aggressively, which
// magnifies source aliasing. Preserve clearly non-poster artwork in full; ordinary portrait covers
// keep the established edge-to-edge poster treatment.
const NON_POSTER_ASPECT_RATIO = 0.9;

export interface PosterRenderLayout {
  sourceX: number;
  sourceY: number;
  sourceWidth: number;
  sourceHeight: number;
  destinationX: number;
  destinationY: number;
  destinationWidth: number;
  destinationHeight: number;
}

export function shouldContainPosterArtwork(naturalWidth: number, naturalHeight: number): boolean {
  return naturalWidth > 0
    && naturalHeight > 0
    && naturalWidth / naturalHeight >= NON_POSTER_ASPECT_RATIO;
}

/**
 * Returns an integer-device-pixel draw layout for a poster canvas. Keeping the destination on
 * device-pixel boundaries avoids asking WebView2 to resample the already downscaled result again.
 */
export function posterRenderLayout(
  sourceWidth: number,
  sourceHeight: number,
  targetWidth: number,
  targetHeight: number,
  contain: boolean,
): PosterRenderLayout | null {
  if (![sourceWidth, sourceHeight, targetWidth, targetHeight].every((value) => Number.isFinite(value) && value > 0)) return null;

  if (contain) {
    const scale = Math.min(targetWidth / sourceWidth, targetHeight / sourceHeight);
    const destinationWidth = Math.max(1, Math.min(targetWidth, Math.round(sourceWidth * scale)));
    const destinationHeight = Math.max(1, Math.min(targetHeight, Math.round(sourceHeight * scale)));
    return {
      sourceX: 0,
      sourceY: 0,
      sourceWidth,
      sourceHeight,
      destinationX: Math.floor((targetWidth - destinationWidth) / 2),
      destinationY: Math.floor((targetHeight - destinationHeight) / 2),
      destinationWidth,
      destinationHeight,
    };
  }

  const sourceAspect = sourceWidth / sourceHeight;
  const targetAspect = targetWidth / targetHeight;
  let croppedWidth = sourceWidth;
  let croppedHeight = sourceHeight;
  if (sourceAspect > targetAspect) croppedWidth = sourceHeight * targetAspect;
  else if (sourceAspect < targetAspect) croppedHeight = sourceWidth / targetAspect;
  return {
    sourceX: (sourceWidth - croppedWidth) / 2,
    sourceY: (sourceHeight - croppedHeight) / 2,
    sourceWidth: croppedWidth,
    sourceHeight: croppedHeight,
    destinationX: 0,
    destinationY: 0,
    destinationWidth: targetWidth,
    destinationHeight: targetHeight,
  };
}

// Bangumi occasionally exposes a landscape or near-square subject image instead of poster art.
// Cropping that image into a tall frame makes WebView upscale its short edge aggressively, which
// magnifies source aliasing. Preserve clearly non-poster artwork in full; ordinary portrait covers
// keep the established edge-to-edge poster treatment.
const NON_POSTER_ASPECT_RATIO = 0.9;

export function shouldContainPosterArtwork(naturalWidth: number, naturalHeight: number): boolean {
  return naturalWidth > 0
    && naturalHeight > 0
    && naturalWidth / naturalHeight >= NON_POSTER_ASPECT_RATIO;
}

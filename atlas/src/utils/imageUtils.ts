import type { AtlasSection } from '../types';

/** Get the recommended downsample level based on zoom */
export function getDownsampleForZoom(zoom: number): number {
  if (zoom >= 3) return 1;
  if (zoom >= 1.5) return 2;
  if (zoom >= 0.7) return 3;
  return 4;
}

/** Get the displayed dimensions of a section image at a given downsample level */
export function getDisplayedSize(section: AtlasSection, downsample: number): { w: number; h: number } {
  const scale = Math.pow(2, downsample);
  return {
    w: Math.ceil(section.width / scale),
    h: Math.ceil(section.height / scale),
  };
}

import type { SvgRegion } from '../types';

/** Parse an Allen Brain Atlas SVG string and extract region paths */
export function parseSvgRegions(svgString: string): {
  regions: SvgRegion[];
  viewBox: { width: number; height: number };
  structureIds: Set<number>;
} {
  const parser = new DOMParser();
  const doc = parser.parseFromString(svgString, 'image/svg+xml');
  const svg = doc.querySelector('svg');

  const width = parseInt(svg?.getAttribute('width') || '0', 10);
  const height = parseInt(svg?.getAttribute('height') || '0', 10);

  const paths = doc.querySelectorAll('path');
  const regions: SvgRegion[] = [];
  const structureIds = new Set<number>();

  paths.forEach((path) => {
    const structureId = parseInt(path.getAttribute('structure_id') || '0', 10);
    const d = path.getAttribute('d') || '';
    const style = path.getAttribute('style') || '';
    const fillMatch = style.match(/fill:(#[0-9a-fA-F]{6})/);
    const fillColor = fillMatch ? fillMatch[1] : '#cccccc';

    if (structureId && d) {
      regions.push({
        pathId: path.getAttribute('id') || `path-${structureId}`,
        structureId,
        d,
        fillColor,
      });
      structureIds.add(structureId);
    }
  });

  return { regions, viewBox: { width, height }, structureIds };
}

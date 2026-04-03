const ALLEN_API = 'https://api.brain-map.org';
const ATLAS_DATASET_ID = 100048576; // Mouse P56 Coronal

/** Get atlas section image URL at given downsample level */
export function getSectionImageUrl(sectionImageId: number, downsample: number = 4): string {
  return `${ALLEN_API}/api/v2/atlas_image_download/${sectionImageId}?downsample=${downsample}&annotation=true`;
}

/** Get section thumbnail URL (high downsample = small image) */
export function getSectionThumbnailUrl(sectionImageId: number): string {
  return getSectionImageUrl(sectionImageId, 6);
}

/** Get SVG annotation URL for a section */
export function getSvgUrl(sectionImageId: number): string {
  return `${ALLEN_API}/api/v2/svg_download/${sectionImageId}?groups=28`;
}

/** Fetch SVG annotation string for a section */
export async function fetchSvgAnnotation(sectionImageId: number): Promise<string> {
  const url = getSvgUrl(sectionImageId);
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`Failed to fetch SVG for section ${sectionImageId}`);
  return resp.text();
}

/**
 * Use Allen's structure_to_image API to find the section image that
 * best shows a given structure. Returns the section_image_id or null.
 */
export async function fetchSectionForStructure(structureId: number): Promise<{
  sectionImageId: number;
  sectionNumber: number;
  x: number;
  y: number;
} | null> {
  const url = `${ALLEN_API}/api/v2/structure_to_image/${ATLAS_DATASET_ID}.json?structure_ids=${structureId}`;
  try {
    const resp = await fetch(url);
    if (!resp.ok) return null;
    const data = await resp.json();
    if (!data.success || !data.msg?.[0]?.image_sync) return null;
    const sync = data.msg[0].image_sync;
    return {
      sectionImageId: sync.section_image_id,
      sectionNumber: sync.section_number,
      x: sync.x,
      y: sync.y,
    };
  } catch {
    return null;
  }
}

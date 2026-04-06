import * as THREE from 'three';
import { OBJLoader } from 'three/examples/jsm/loaders/OBJLoader.js';

const PRE_FETCHED_BASE = '/meshes';
const ALLEN_PROXY_BASE = '/allen-meshes';

const loader = new OBJLoader();

/**
 * Load an OBJ mesh for a given structure ID.
 * Tries pre-fetched static file first, then falls back to proxied Allen URL.
 */
export async function loadObjMesh(structureId: number): Promise<THREE.BufferGeometry | null> {
  // Try pre-fetched first
  const staticUrl = `${PRE_FETCHED_BASE}/${structureId}.obj`;
  try {
    const geo = await fetchAndParse(staticUrl);
    if (geo) return geo;
  } catch { /* fall through */ }

  // Fallback to proxied Allen URL
  const proxyUrl = `${ALLEN_PROXY_BASE}/${structureId}.obj`;
  try {
    const geo = await fetchAndParse(proxyUrl);
    if (geo) return geo;
  } catch { /* fall through */ }

  return null;
}

async function fetchAndParse(url: string): Promise<THREE.BufferGeometry | null> {
  return new Promise((resolve, reject) => {
    loader.load(
      url,
      (group) => {
        // OBJLoader returns a Group; extract the first mesh geometry
        let geo: THREE.BufferGeometry | null = null;
        group.traverse((child) => {
          if (!geo && (child as THREE.Mesh).isMesh) {
            geo = (child as THREE.Mesh).geometry as THREE.BufferGeometry;
          }
        });

        if (geo !== null) {
          const g = geo as THREE.BufferGeometry;
          // Compute normals if missing
          if (!g.attributes.normal) {
            g.computeVertexNormals();
          }
          g.computeBoundingBox();
          g.computeBoundingSphere();
          resolve(g);
        } else {
          resolve(null);
        }
      },
      undefined,
      (err) => reject(err),
    );
  });
}

/**
 * Get the center and size of the brain bounding box for normalization.
 * Allen CCF coordinates: ~13200 x 8000 x 11400 (in 10um voxels)
 */
export const BRAIN_CENTER = new THREE.Vector3(6600, 4000, 5700);
export const BRAIN_SCALE = 0.001; // Scale down from voxel coords to manageable scene units

/**
 * Build-time script to download Allen CCF 2017 structure meshes (OBJ format).
 * Reads all structure IDs from the ontology and attempts to download each one.
 * Allen only has meshes for ~700 structures; 404s are silently skipped.
 *
 * Usage: npx tsx scripts/fetch-meshes.ts
 */

import { writeFileSync, readFileSync, existsSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import https from 'node:https';
import http from 'node:http';

const MESH_BASE =
  'http://download.alleninstitute.org/informatics-archive/current-release/mouse_ccf/annotation/ccf_2017/structure_meshes';

const OUTPUT_DIR = join(import.meta.dirname ?? '.', '..', 'public', 'meshes');
const ONTOLOGY_PATH = join(import.meta.dirname ?? '.', '..', 'src', 'data', 'ontology.json');

interface OntologyNode {
  id: number;
  ch?: OntologyNode[];
}

function collectAllIds(node: OntologyNode): number[] {
  const ids = [node.id];
  if (node.ch) {
    for (const child of node.ch) {
      ids.push(...collectAllIds(child));
    }
  }
  return ids;
}

function download(url: string): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const client = url.startsWith('https') ? https : http;
    client.get(url, { headers: { 'User-Agent': 'CortexMap-Atlas/1.0' } }, (res) => {
      if (res.statusCode === 301 || res.statusCode === 302) {
        const location = res.headers.location;
        if (location) return download(location).then(resolve).catch(reject);
      }
      if (res.statusCode !== 200) {
        reject(new Error(`HTTP ${res.statusCode}`));
        res.resume();
        return;
      }
      const chunks: Buffer[] = [];
      res.on('data', (chunk) => chunks.push(chunk));
      res.on('end', () => resolve(Buffer.concat(chunks)));
      res.on('error', reject);
    }).on('error', reject);
  });
}

async function main() {
  if (!existsSync(OUTPUT_DIR)) {
    mkdirSync(OUTPUT_DIR, { recursive: true });
  }

  // Read all structure IDs from ontology
  const ontology: OntologyNode = JSON.parse(readFileSync(ONTOLOGY_PATH, 'utf-8'));
  const allIds = collectAllIds(ontology);
  console.log(`Found ${allIds.length} structures in ontology. Downloading meshes to ${OUTPUT_DIR}`);

  let success = 0;
  let skipped = 0;
  let failed = 0;

  // Process in batches of 10 to avoid overwhelming the server
  const BATCH_SIZE = 10;
  for (let i = 0; i < allIds.length; i += BATCH_SIZE) {
    const batch = allIds.slice(i, i + BATCH_SIZE);
    await Promise.allSettled(
      batch.map(async (id) => {
        const outPath = join(OUTPUT_DIR, `${id}.obj`);
        if (existsSync(outPath)) {
          skipped++;
          return;
        }

        const url = `${MESH_BASE}/${id}.obj`;
        try {
          const data = await download(url);
          writeFileSync(outPath, data);
          const sizeKB = (data.length / 1024).toFixed(1);
          console.log(`  [ok]   ${id}.obj (${sizeKB} KB)`);
          success++;
        } catch (err: any) {
          if (err.message.includes('404')) {
            // No mesh for this structure -- expected for many
            failed++;
          } else {
            console.log(`  [fail] ${id}.obj - ${err.message}`);
            failed++;
          }
        }
      })
    );

    // Progress every 100
    if ((i + BATCH_SIZE) % 100 === 0 || i + BATCH_SIZE >= allIds.length) {
      const progress = Math.min(i + BATCH_SIZE, allIds.length);
      console.log(`  ... ${progress}/${allIds.length} checked (${success} downloaded, ${skipped} cached, ${failed} no mesh)`);
    }
  }

  console.log(`\nDone: ${success} new, ${skipped} cached, ${failed} no mesh available (total: ${allIds.length})`);
}

await main();

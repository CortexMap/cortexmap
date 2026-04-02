/**
 * Build-time data extraction script.
 * Fetches the Allen Brain Atlas ontology tree and saves it as static JSON.
 * 
 * Usage: npx tsx scripts/fetch-atlas-data.ts
 */
import https from 'node:https';
import fs from 'node:fs';
import path from 'node:path';

const ONTOLOGY_URL = 'https://api.brain-map.org/api/v2/structure_graph_download/1.json';
const OUTPUT_DIR = path.join(import.meta.dirname ?? __dirname, '..', 'src', 'data');

function fetchJSON(url: string): Promise<string> {
  return new Promise((resolve, reject) => {
    https.get(url, (res) => {
      let data = '';
      res.on('data', (chunk: Buffer) => { data += chunk.toString(); });
      res.on('end', () => resolve(data));
      res.on('error', reject);
    }).on('error', reject);
  });
}

interface AllenNode {
  id: number;
  acronym: string;
  name: string;
  color_hex_triplet: string;
  graph_order: number;
  st_level: number;
  parent_structure_id: number | null;
  children: AllenNode[];
}

interface SlimNode {
  id: number;
  a: string; // acronym
  n: string; // name
  c: string; // color hex
  o: number; // graph_order
  l: number; // st_level
  p: number | null; // parent id
  ch: SlimNode[];
}

function slimTree(node: AllenNode): SlimNode {
  return {
    id: node.id,
    a: node.acronym,
    n: node.name,
    c: node.color_hex_triplet,
    o: node.graph_order,
    l: node.st_level,
    p: node.parent_structure_id,
    ch: (node.children || []).map(slimTree),
  };
}

async function main() {
  console.log('Fetching Allen Brain Atlas ontology...');
  const raw = await fetchJSON(ONTOLOGY_URL);
  const parsed = JSON.parse(raw);

  if (!parsed.success || !parsed.msg?.[0]) {
    console.error('Failed to fetch ontology:', parsed);
    process.exit(1);
  }

  const root: AllenNode = parsed.msg[0];
  const slim = slimTree(root);

  fs.mkdirSync(OUTPUT_DIR, { recursive: true });
  const outPath = path.join(OUTPUT_DIR, 'ontology.json');
  fs.writeFileSync(outPath, JSON.stringify(slim));
  
  const stats = fs.statSync(outPath);
  console.log(`Ontology saved to ${outPath} (${(stats.size / 1024).toFixed(0)} KB)`);
}

main().catch((err) => {
  console.error('Error:', err);
  process.exit(1);
});

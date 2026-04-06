/** Allen Brain Atlas section image metadata */
export interface AtlasSection {
  id: number;
  section_number: number;
  width: number;
  height: number;
  /** Offset X within the image sheet (for imageservice crop) */
  x?: number;
  /** Offset Y within the image sheet (for imageservice crop) */
  y?: number;
  /** Path to the raw Nissl .aff file on Allen's imageservice */
  path?: string;
  /** Path to the "Atlas - Adult Mouse" rendered .aff file */
  adult_path?: string;
  /** Path to the "Atlas - Developing Mouse" rendered .aff file */
  dev_path?: string;
}

/** Slim ontology node (as stored in ontology.json) */
export interface OntologyNode {
  id: number;
  a: string; // acronym
  n: string; // name
  c: string; // color_hex_triplet
  o: number; // graph_order
  l: number; // st_level
  p: number | null; // parent_structure_id
  ch: OntologyNode[];
}

/** Flat representation for virtualized tree rendering */
export interface FlatTreeNode {
  id: number;
  acronym: string;
  name: string;
  color: string;
  graphOrder: number;
  stLevel: number;
  parentId: number | null;
  depth: number;
  hasChildren: boolean;
  ancestorPath: number[]; // ids from root to this node's parent
}

/** SVG path region extracted from Allen SVG */
export interface SvgRegion {
  pathId: string;
  structureId: number;
  d: string; // SVG path data
  fillColor: string;
  strokeColor: string;
}

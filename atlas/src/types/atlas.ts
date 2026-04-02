/** Allen Brain Atlas section image metadata */
export interface AtlasSection {
  id: number;
  section_number: number;
  width: number;
  height: number;
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
}

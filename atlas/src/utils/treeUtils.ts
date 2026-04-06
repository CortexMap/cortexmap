import type { OntologyNode, FlatTreeNode } from '../types';
import type { CortexmapRegion } from '../types/cortexmap';

/**
 * Build an OntologyNode tree from a flat list of CortexmapRegion objects.
 * Regions with no parent (parent_region_id === null) or whose parent is not
 * in the list become roots. If there are multiple roots they are gathered
 * under a synthetic "root" node.
 */
export function buildTreeFromRegions(regions: CortexmapRegion[]): OntologyNode {
  // Build a map of region_id -> OntologyNode (without children yet)
  const nodeMap = new Map<number, OntologyNode>();
  for (const r of regions) {
    const colorHex = r.color
      ? [r.color.red, r.color.green, r.color.blue]
          .map((c) => c.toString(16).padStart(2, '0'))
          .join('')
      : '888888';

    nodeMap.set(r.region_id, {
      id: r.region_id,
      a: r.acronym || '',
      n: r.name,
      c: colorHex,
      o: r.structure_order || 0,
      l: 0, // will compute depth later
      p: r.parent_region_id,
      ch: [],
    });
  }

  // Wire children to parents
  const roots: OntologyNode[] = [];
  for (const node of nodeMap.values()) {
    if (node.p !== null && nodeMap.has(node.p)) {
      nodeMap.get(node.p)!.ch.push(node);
    } else {
      roots.push(node);
    }
  }

  // Sort children by structure_order at every level
  function sortChildren(node: OntologyNode) {
    node.ch.sort((a, b) => a.o - b.o);
    node.ch.forEach(sortChildren);
  }

  // Compute st_level (depth in tree)
  function assignLevels(node: OntologyNode, depth: number) {
    node.l = depth;
    node.ch.forEach((c) => assignLevels(c, depth + 1));
  }

  // If single root, use it directly; otherwise wrap in synthetic root
  let root: OntologyNode;
  if (roots.length === 1) {
    root = roots[0];
  } else {
    root = {
      id: 997, // Allen "root" brain id
      a: 'root',
      n: 'Brain Regions',
      c: '888888',
      o: 0,
      l: 0,
      p: null,
      ch: roots,
    };
  }

  sortChildren(root);
  assignLevels(root, 0);

  return root;
}

/** Flatten ontology tree into a list suitable for virtualized rendering */
export function flattenTree(
  node: OntologyNode,
  expandedNodes: Set<number>,
  depth: number = 0,
  ancestorPath: number[] = []
): FlatTreeNode[] {
  const flat: FlatTreeNode = {
    id: node.id,
    acronym: node.a,
    name: node.n,
    color: node.c,
    graphOrder: node.o,
    stLevel: node.l,
    parentId: node.p,
    depth,
    hasChildren: node.ch.length > 0,
    ancestorPath,
  };

  const result: FlatTreeNode[] = [flat];

  if (expandedNodes.has(node.id) && node.ch.length > 0) {
    const childPath = [...ancestorPath, node.id];
    for (const child of node.ch) {
      result.push(...flattenTree(child, expandedNodes, depth + 1, childPath));
    }
  }

  return result;
}

/** Find a node by ID in the ontology tree */
export function findNode(root: OntologyNode, id: number): OntologyNode | null {
  if (root.id === id) return root;
  for (const child of root.ch) {
    const found = findNode(child, id);
    if (found) return found;
  }
  return null;
}

/** Get ancestor path from root to a given node */
export function getAncestorPath(root: OntologyNode, targetId: number): number[] {
  const path: number[] = [];
  function dfs(node: OntologyNode): boolean {
    if (node.id === targetId) {
      return true;
    }
    for (const child of node.ch) {
      if (dfs(child)) {
        path.unshift(node.id);
        return true;
      }
    }
    return false;
  }
  dfs(root);
  return path;
}

/** Collect all node IDs and their st_levels for level expansion */
export function collectAllNodes(node: OntologyNode): { id: number; stLevel: number; parentId: number | null }[] {
  const result: { id: number; stLevel: number; parentId: number | null }[] = [
    { id: node.id, stLevel: node.l, parentId: node.p },
  ];
  for (const child of node.ch) {
    result.push(...collectAllNodes(child));
  }
  return result;
}

/** Filter tree nodes by search query, returning IDs of matching nodes and their ancestors */
export function searchTree(
  root: OntologyNode,
  query: string
): { matchIds: Set<number>; expandIds: Set<number> } {
  const q = query.toLowerCase();
  const matchIds = new Set<number>();
  const expandIds = new Set<number>();

  function dfs(node: OntologyNode, ancestors: number[]): boolean {
    const matches =
      node.n.toLowerCase().includes(q) || node.a.toLowerCase().includes(q);

    let childMatches = false;
    for (const child of node.ch) {
      if (dfs(child, [...ancestors, node.id])) {
        childMatches = true;
      }
    }

    if (matches) {
      matchIds.add(node.id);
      ancestors.forEach((a) => expandIds.add(a));
    }

    if (childMatches) {
      expandIds.add(node.id);
    }

    return matches || childMatches;
  }

  dfs(root, []);
  return { matchIds, expandIds };
}

/** Collect all descendant IDs (inclusive) from a given node */
export function collectDescendantIds(node: OntologyNode): number[] {
  const ids: number[] = [node.id];
  for (const child of node.ch) {
    ids.push(...collectDescendantIds(child));
  }
  return ids;
}

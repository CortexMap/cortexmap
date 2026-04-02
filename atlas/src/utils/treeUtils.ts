import type { OntologyNode, FlatTreeNode } from '../types';

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

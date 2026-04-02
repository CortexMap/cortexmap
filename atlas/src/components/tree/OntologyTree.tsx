import { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import { useAtlasStore } from '../../store/atlasStore';
import { flattenTree, getAncestorPath, searchTree, collectAllNodes } from '../../utils/treeUtils';
import type { OntologyNode, FlatTreeNode } from '../../types';
import styles from './OntologyTree.module.css';

const MAX_VISIBLE = 300;

export function OntologyTree() {
  const {
    ontology, selectedStructureId, hoveredStructureId,
    setHovered, selectStructure, annotatedStructures,
  } = useAtlasStore();

  const [searchQuery, setSearchQuery] = useState('');
  const [expanded, setExpanded] = useState<Set<number>>(new Set([997, 8, 567, 343, 512]));
  const listRef = useRef<HTMLDivElement>(null);
  const selectedRef = useRef<HTMLDivElement>(null);

  // Auto-expand path to selected structure
  useEffect(() => {
    if (selectedStructureId !== null && ontology) {
      const path = getAncestorPath(ontology, selectedStructureId);
      if (path.length > 0) {
        setExpanded((prev) => {
          const next = new Set(prev);
          for (const id of path) next.add(id);
          return next;
        });
        requestAnimationFrame(() => {
          selectedRef.current?.scrollIntoView({ behavior: 'smooth', block: 'center' });
        });
      }
    }
  }, [selectedStructureId, ontology]);

  // Handle search: auto-expand matching nodes
  const searchResult = useMemo(() => {
    if (!ontology || !searchQuery.trim()) return null;
    return searchTree(ontology, searchQuery.trim());
  }, [ontology, searchQuery]);

  // Flatten tree with expansion state
  const flatNodes = useMemo(() => {
    if (!ontology) return [];
    const effectiveExpanded = searchResult
      ? new Set([...expanded, ...searchResult.expandIds])
      : expanded;
    return flattenTree(ontology, effectiveExpanded);
  }, [ontology, expanded, searchResult]);

  // Filter by search if active
  const visibleNodes = useMemo(() => {
    if (!searchResult) return flatNodes;
    return flatNodes.filter((n) =>
      searchResult.matchIds.has(n.id) || searchResult.expandIds.has(n.id)
    );
  }, [flatNodes, searchResult]);

  const toggleExpand = useCallback((id: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const expandAll = useCallback(() => {
    if (!ontology) return;
    const all = collectAllNodes(ontology);
    setExpanded(new Set(all.map((n) => n.id)));
  }, [ontology]);

  const collapseAll = useCallback(() => setExpanded(new Set()), []);

  if (!ontology) {
    return <div className={styles.container}><div className={styles.loading}>Loading ontology...</div></div>;
  }

  return (
    <div className={styles.container}>
      {/* Search */}
      <div className={styles.searchBar}>
        <input
          type="text"
          placeholder="Search regions..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className={styles.searchInput}
        />
        {searchQuery && (
          <button className={styles.clearBtn} onClick={() => setSearchQuery('')}>&times;</button>
        )}
      </div>

      {/* Controls */}
      <div className={styles.controls}>
        <div className={styles.expandBtns}>
          <button className={styles.controlBtn} onClick={expandAll} title="Expand all">Expand</button>
          <button className={styles.controlBtn} onClick={collapseAll} title="Collapse all">Collapse</button>
        </div>
      </div>

      {/* Count */}
      <div className={styles.count}>{visibleNodes.length} regions</div>

      {/* Tree list */}
      <div className={styles.treeList} ref={listRef}>
        {visibleNodes.slice(0, MAX_VISIBLE).map((flat) => (
          <TreeRow
            key={flat.id}
            flat={flat}
            isSelected={flat.id === selectedStructureId}
            isHovered={flat.id === hoveredStructureId}
            isAnnotated={annotatedStructures.has(flat.id)}
            isExpanded={expanded.has(flat.id)}
            isMatch={searchResult ? searchResult.matchIds.has(flat.id) : false}
            onToggle={toggleExpand}
            onSelect={selectStructure}
            onHover={setHovered}
            selectedRef={flat.id === selectedStructureId ? selectedRef : undefined}
          />
        ))}
        {visibleNodes.length > MAX_VISIBLE && (
          <div className={styles.overflow}>
            +{visibleNodes.length - MAX_VISIBLE} more... (refine search)
          </div>
        )}
      </div>
    </div>
  );
}

interface TreeRowProps {
  flat: FlatTreeNode;
  isSelected: boolean;
  isHovered: boolean;
  isAnnotated: boolean;
  isExpanded: boolean;
  isMatch: boolean;
  onToggle: (id: number) => void;
  onSelect: (id: number) => void;
  onHover: (id: number | null) => void;
  selectedRef?: React.Ref<HTMLDivElement>;
}

function TreeRow({ flat, isSelected, isHovered, isAnnotated, isExpanded, isMatch, onToggle, onSelect, onHover, selectedRef }: TreeRowProps) {
  const cls = [
    styles.row,
    isSelected ? styles.selected : '',
    isHovered ? styles.hovered : '',
    isAnnotated ? styles.annotated : '',
    isMatch ? styles.match : '',
  ].join(' ');

  return (
    <div
      ref={selectedRef}
      className={cls}
      style={{ paddingLeft: 8 + flat.depth * 16 }}
      onMouseEnter={() => onHover(flat.id)}
      onMouseLeave={() => onHover(null)}
      onClick={() => onSelect(flat.id)}
    >
      {flat.hasChildren ? (
        <button
          className={styles.toggle}
          onClick={(e) => { e.stopPropagation(); onToggle(flat.id); }}
        >
          {isExpanded ? '\u25BE' : '\u25B8'}
        </button>
      ) : (
        <span className={styles.togglePlaceholder} />
      )}
      <span
        className={styles.colorDot}
        style={{ backgroundColor: flat.color ? `#${flat.color}` : '#475569' }}
      />
      <span className={styles.acronymLabel}>{flat.acronym}</span>
      <span className={styles.nameLabel}>{flat.name}</span>
    </div>
  );
}

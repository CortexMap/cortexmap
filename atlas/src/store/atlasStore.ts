import { create } from 'zustand';
import type { CortexmapRegion, RegionSummary, RegionStatus, OntologyNode, AtlasSection } from '../types';
import { fetchSectionForStructure } from '../api/allen';
import sectionsData from '../data/sections.json';

const sections = sectionsData as AtlasSection[];

interface AtlasStore {
  // Ontology
  ontology: OntologyNode | null;
  ontologyLoading: boolean;

  // Viewer state
  currentSectionIndex: number;
  zoomLevel: number;
  panX: number;
  panY: number;
  imageLoaded: boolean;

  // Selection state
  selectedStructureId: number | null;
  hoveredStructureId: number | null;
  highlightedPath: number[];

  // Tree state
  expandedNodes: Set<number>;
  treeSearchQuery: string;
  annotatedStructures: Set<number>;

  // Section annotation cache: sectionIndex -> Set of structureIds
  sectionAnnotationCache: Map<number, Set<number>>;
  navigating: boolean;

  // Cortexmap data
  cortexmapRegions: CortexmapRegion[];
  cortexmapRegionMap: Map<number, CortexmapRegion>;
  regionStatus: Map<string, RegionStatus>;
  regionSummaries: Map<string, RegionSummary[]>;
  cortexmapLoaded: boolean;

  // Sections
  sections: AtlasSection[];

  // Ontology actions
  loadOntology: () => Promise<void>;
  loadSections: () => void;

  // Viewer actions
  setCurrentSection: (index: number) => void;
  setCurrentSectionIndex: (index: number) => void;
  nextSection: () => void;
  prevSection: () => void;
  setZoom: (zoom: number) => void;
  setPan: (x: number, y: number) => void;
  resetView: () => void;
  setImageLoaded: (loaded: boolean) => void;

  // Selection actions
  selectStructure: (id: number | null) => void;
  setHovered: (id: number | null) => void;
  hoverStructure: (id: number | null) => void;
  setHighlightedPath: (path: number[]) => void;

  // Cache actions
  cacheSectionAnnotations: (sectionIndex: number, structureIds: Set<number>) => void;
  setAnnotatedStructures: (ids: Set<number>) => void;
  navigateToStructure: (structureId: number) => Promise<void>;

  // Tree actions
  toggleNode: (id: number) => void;
  expandNodes: (ids: number[]) => void;
  collapseAll: () => void;
  expandToLevel: (level: number, allNodes: { id: number; stLevel: number; parentId: number | null }[]) => void;
  setTreeSearch: (query: string) => void;

  // Cortexmap actions
  setCortexmapRegions: (regions: CortexmapRegion[]) => void;
  setRegionStatus: (uuid: string, status: RegionStatus) => void;
  setRegionSummaries: (uuid: string, summaries: RegionSummary[]) => void;
}

export const useAtlasStore = create<AtlasStore>((set, get) => ({
  // Ontology
  ontology: null,
  ontologyLoading: false,

  // Initial viewer state
  currentSectionIndex: 44,
  zoomLevel: 1,
  panX: 0,
  panY: 0,
  imageLoaded: false,

  // Initial selection
  selectedStructureId: null,
  hoveredStructureId: null,
  highlightedPath: [],

  // Initial tree
  expandedNodes: new Set([997, 8, 567, 343, 512]),
  treeSearchQuery: '',
  annotatedStructures: new Set(),

  // Section annotation cache
  sectionAnnotationCache: new Map(),
  navigating: false,

  // Initial cortexmap
  cortexmapRegions: [],
  cortexmapRegionMap: new Map(),
  regionStatus: new Map(),
  regionSummaries: new Map(),
  cortexmapLoaded: false,

  sections,

  // Ontology actions
  loadOntology: async () => {
    if (get().ontology || get().ontologyLoading) return;
    set({ ontologyLoading: true });
    try {
      const data = await import('../data/ontology.json');
      set({ ontology: data.default as OntologyNode, ontologyLoading: false });
    } catch (err) {
      console.error('Failed to load ontology:', err);
      set({ ontologyLoading: false });
    }
  },

  loadSections: () => {},

  // Viewer actions
  setCurrentSection: (index) => set({ currentSectionIndex: index, imageLoaded: false }),
  setCurrentSectionIndex: (index) => set({ currentSectionIndex: index, imageLoaded: false }),
  nextSection: () => {
    const { currentSectionIndex } = get();
    if (currentSectionIndex < sections.length - 1) {
      set({ currentSectionIndex: currentSectionIndex + 1, imageLoaded: false });
    }
  },
  prevSection: () => {
    const { currentSectionIndex } = get();
    if (currentSectionIndex > 0) {
      set({ currentSectionIndex: currentSectionIndex - 1, imageLoaded: false });
    }
  },
  setZoom: (zoom) => set({ zoomLevel: Math.max(0.3, Math.min(10, zoom)) }),
  setPan: (x, y) => set({ panX: x, panY: y }),
  resetView: () => set({ zoomLevel: 1, panX: 0, panY: 0 }),
  setImageLoaded: (loaded) => set({ imageLoaded: loaded }),

  // Selection actions
  selectStructure: (id) => {
    set({ selectedStructureId: id });
    if (id !== null) {
      // Check if the current section has this structure
      const { annotatedStructures } = get();
      if (!annotatedStructures.has(id)) {
        // Navigate to a section that contains this structure
        get().navigateToStructure(id);
      }
    }
  },
  setHovered: (id) => set({ hoveredStructureId: id }),
  hoverStructure: (id) => set({ hoveredStructureId: id }),
  setHighlightedPath: (path) => set({ highlightedPath: path }),

  // Cache actions
  cacheSectionAnnotations: (sectionIndex, structureIds) => {
    const cache = new Map(get().sectionAnnotationCache);
    cache.set(sectionIndex, structureIds);
    set({ sectionAnnotationCache: cache });
  },
  setAnnotatedStructures: (ids) => set({ annotatedStructures: ids }),

  navigateToStructure: async (structureId: number) => {
    const state = get();
    if (state.navigating) return;

    // 1. Check cache first — if we already know which section has this structure
    for (const [sectionIdx, structs] of state.sectionAnnotationCache) {
      if (structs.has(structureId)) {
        set({ currentSectionIndex: sectionIdx, imageLoaded: false });
        return;
      }
    }

    // 2. Use Allen's structure_to_image API for direct lookup
    set({ navigating: true });
    try {
      const result = await fetchSectionForStructure(structureId);
      if (result) {
        const idx = sections.findIndex((s) => s.id === result.sectionImageId);
        if (idx !== -1) {
          set({ currentSectionIndex: idx, imageLoaded: false, navigating: false });
          return;
        }
      }
    } catch (err) {
      console.error('Failed to fetch section for structure:', err);
    }
    // Structure not found in any section
    set({ navigating: false });
  },

  // Tree actions
  toggleNode: (id) => {
    const expanded = new Set(get().expandedNodes);
    if (expanded.has(id)) expanded.delete(id);
    else expanded.add(id);
    set({ expandedNodes: expanded });
  },
  expandNodes: (ids) => {
    const expanded = new Set(get().expandedNodes);
    ids.forEach((id) => expanded.add(id));
    set({ expandedNodes: expanded });
  },
  collapseAll: () => set({ expandedNodes: new Set([997]) }),
  expandToLevel: (level, allNodes) => {
    const expanded = new Set<number>();
    allNodes.forEach((node) => {
      if (node.stLevel <= level) expanded.add(node.id);
    });
    set({ expandedNodes: expanded });
  },
  setTreeSearch: (query) => set({ treeSearchQuery: query }),

  // Cortexmap actions
  setCortexmapRegions: (regions) => {
    const map = new Map<number, CortexmapRegion>();
    regions.forEach((r) => map.set(r.region_id, r));
    set({ cortexmapRegions: regions, cortexmapRegionMap: map, cortexmapLoaded: true });
  },
  setRegionStatus: (uuid, status) => {
    const newMap = new Map(get().regionStatus);
    newMap.set(uuid, status);
    set({ regionStatus: newMap });
  },
  setRegionSummaries: (uuid, summaries) => {
    const newMap = new Map(get().regionSummaries);
    newMap.set(uuid, summaries);
    set({ regionSummaries: newMap });
  },
}));

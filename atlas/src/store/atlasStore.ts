import { create } from 'zustand';
import type { CortexmapRegion, RegionSummary, RegionStatus, OntologyNode, AtlasSection } from '../types';
import { fetchSectionForStructure, DATASET_CORONAL, DATASET_SAGITTAL } from '../api/allen';
import coronalSectionsData from '../data/sections.json';
import sagittalSectionsData from '../data/sections-sagittal.json';

const coronalSections = coronalSectionsData as AtlasSection[];
const sagittalSections = sagittalSectionsData as AtlasSection[];

export type ViewPlane = 'coronal' | 'sagittal';

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
  viewPlane: ViewPlane;

  // Ontology actions
  loadOntology: () => Promise<void>;
  loadSections: () => void;
  setViewPlane: (plane: ViewPlane) => void;

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

  // 3D viewer state (BufferGeometry from three.js, typed as any to avoid eager import)
  loadedMeshes: Map<number, any>;
  visibleMeshIds: Set<number>;
  meshOpacity: number;
  viewer3dMode: 'regions' | 'wireframe';
  meshesLoading: boolean;
  meshLoadProgress: { loaded: number; total: number };
  autoRotate: boolean;
  highlight3d: boolean;
  focused3dRegionId: number | null;
  focused3dMeshIds: Set<number>;
  checked3dIds: Set<number>; // multi-select checkboxes in tree for 3D visibility
  fallback3d: { regionName: string; parentName: string; parentId: number } | null; // toast info when falling back to parent mesh

  // 3D viewer actions
  loadMesh: (structureId: number) => Promise<any | null>;
  loadInitialMeshes: () => Promise<void>;
  toggleMeshVisibility: (structureId: number) => void;
  setMeshOpacity: (opacity: number) => void;
  setViewer3dMode: (mode: 'regions' | 'wireframe') => void;
  setAutoRotate: (autoRotate: boolean) => void;
  setHighlight3d: (highlight: boolean) => void;
  focusOn3dRegion: (structureId: number) => Promise<void>;
  clearFocus3d: () => void;
  check3dRegion: (structureId: number) => Promise<void>;
  uncheck3dRegion: (structureId: number) => void;
  clearAllChecked3d: () => void;
  clearFallback3d: () => void;
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

  sections: coronalSections,
  viewPlane: 'coronal' as ViewPlane,

  // Ontology actions
  loadOntology: async () => {
    if (get().ontology || get().ontologyLoading) return;
    set({ ontologyLoading: true });
    try {
      // Fetch regions from the project API (orch) instead of Allen ontology.json
      const { fetchAllRegions } = await import('../api/cortexmap');
      const { buildTreeFromRegions } = await import('../utils/treeUtils');
      const regions = await fetchAllRegions();

      // Build the ontology tree from the flat region list
      const tree = buildTreeFromRegions(regions);

      // Also populate the cortexmap region map for detail panel lookups
      const cortexmapRegionMap = new Map<number, CortexmapRegion>();
      for (const r of regions) {
        cortexmapRegionMap.set(r.region_id, r);
      }

      set({
        ontology: tree,
        ontologyLoading: false,
        cortexmapRegions: regions,
        cortexmapRegionMap,
        cortexmapLoaded: true,
      });
    } catch (err) {
      console.error('Failed to load regions from API:', err);
      // Fallback to static ontology.json if API fails
      try {
        const data = await import('../data/ontology.json');
        set({ ontology: data.default as OntologyNode, ontologyLoading: false });
      } catch (fallbackErr) {
        console.error('Fallback ontology load also failed:', fallbackErr);
        set({ ontologyLoading: false });
      }
    }
  },

  loadSections: () => {},

  setViewPlane: (plane: ViewPlane) => {
    const newSections = plane === 'coronal' ? coronalSections : sagittalSections;
    set({
      viewPlane: plane,
      sections: newSections,
      currentSectionIndex: Math.floor(newSections.length / 3), // start at ~1/3 for a reasonable default
      imageLoaded: false,
      annotatedStructures: new Set(),
      sectionAnnotationCache: new Map(),
      zoomLevel: 1,
      panX: 0,
      panY: 0,
    });
    // If a structure is selected, navigate to its section in the new plane
    const { selectedStructureId } = get();
    if (selectedStructureId !== null) {
      get().navigateToStructure(selectedStructureId);
    }
  },

  // Viewer actions
  setCurrentSection: (index) => set({ currentSectionIndex: index, imageLoaded: false }),
  setCurrentSectionIndex: (index) => set({ currentSectionIndex: index, imageLoaded: false }),
  nextSection: () => {
    const { currentSectionIndex, sections: currentSections } = get();
    if (currentSectionIndex < currentSections.length - 1) {
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

    const currentSections = state.sections;
    const datasetId = state.viewPlane === 'coronal' ? DATASET_CORONAL : DATASET_SAGITTAL;

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
      const result = await fetchSectionForStructure(structureId, datasetId);
      if (result) {
        const idx = currentSections.findIndex((s) => s.id === result.sectionImageId);
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

  // 3D viewer state
  loadedMeshes: new Map(),
  visibleMeshIds: new Set(),
  meshOpacity: 0.7,
  viewer3dMode: 'regions',
  meshesLoading: false,
  meshLoadProgress: { loaded: 0, total: 0 },
  autoRotate: false,
  highlight3d: true,
  focused3dRegionId: null,
  focused3dMeshIds: new Set(),
  checked3dIds: new Set(),
  fallback3d: null,

  // 3D viewer actions
  loadMesh: async (structureId: number) => {
    const existing = get().loadedMeshes.get(structureId);
    if (existing) return existing;

    // Dynamic import to avoid bundling Three.js in the main chunk
    const { loadObjMesh } = await import('../utils/objLoader');
    const geometry = await loadObjMesh(structureId);
    if (geometry) {
      const meshes = new Map(get().loadedMeshes);
      meshes.set(structureId, geometry);
      const visible = new Set(get().visibleMeshIds);
      visible.add(structureId);
      set({ loadedMeshes: meshes, visibleMeshIds: visible });
    }
    return geometry;
  },

  loadInitialMeshes: async () => {
    if (get().meshesLoading) return;
    set({ meshesLoading: true });

    // Brain shell + major regions
    const ids = [997, 8, 567, 343, 512, 1009, 73, 315, 698, 1089, 549, 623, 803, 1097, 1065, 313, 771, 354];
    set({ meshLoadProgress: { loaded: 0, total: ids.length } });

    let loaded = 0;
    // Load in batches of 4 for better parallelism
    for (let i = 0; i < ids.length; i += 4) {
      const batch = ids.slice(i, i + 4);
      await Promise.all(batch.map((id) => get().loadMesh(id).catch(() => null)));
      loaded += batch.length;
      set({ meshLoadProgress: { loaded: Math.min(loaded, ids.length), total: ids.length } });
    }

    set({ meshesLoading: false });
  },

  toggleMeshVisibility: (structureId: number) => {
    const visible = new Set(get().visibleMeshIds);
    if (visible.has(structureId)) visible.delete(structureId);
    else visible.add(structureId);
    set({ visibleMeshIds: visible });
  },

  setMeshOpacity: (opacity: number) => set({ meshOpacity: opacity }),
  setViewer3dMode: (mode) => set({ viewer3dMode: mode }),
  setAutoRotate: (autoRotate) => set({ autoRotate }),
  setHighlight3d: (highlight) => set({ highlight3d: highlight }),

  focusOn3dRegion: async (structureId: number) => {
    const { ontology } = get();
    if (!ontology) return;

    const { findNode, collectDescendantIds } = await import('../utils/treeUtils');
    const node = findNode(ontology, structureId);
    if (!node) return;

    const descendantIds = collectDescendantIds(node);
    const focusedSet = new Set(descendantIds);

    set({
      focused3dRegionId: structureId,
      focused3dMeshIds: focusedSet,
      meshesLoading: true,
      meshLoadProgress: { loaded: 0, total: descendantIds.length },
    });

    let loaded = 0;
    for (let i = 0; i < descendantIds.length; i += 6) {
      const batch = descendantIds.slice(i, i + 6);
      await Promise.all(batch.map((id) => get().loadMesh(id).catch(() => null)));
      loaded += batch.length;
      set({ meshLoadProgress: { loaded: Math.min(loaded, descendantIds.length), total: descendantIds.length } });
    }

    const visible = new Set<number>();
    for (const id of descendantIds) {
      if (get().loadedMeshes.has(id)) visible.add(id);
    }
    set({ visibleMeshIds: visible, meshesLoading: false });
  },

  clearFocus3d: () => {
    const MAJOR = [997, 8, 567, 343, 512, 1009, 73, 315, 698, 1089, 549, 623, 803, 1097, 1065, 313, 771, 354];
    const visible = new Set<number>();
    for (const id of MAJOR) {
      if (get().loadedMeshes.has(id)) visible.add(id);
    }
    set({
      focused3dRegionId: null,
      focused3dMeshIds: new Set(),
      checked3dIds: new Set(),
      visibleMeshIds: visible,
    });
  },

  check3dRegion: async (structureId: number) => {
    const { ontology } = get();
    if (!ontology) return;

    const { findNode, collectDescendantIds } = await import('../utils/treeUtils');
    const node = findNode(ontology, structureId);
    if (!node) return;

    const descendantIds = collectDescendantIds(node);
    const checked = new Set(get().checked3dIds);
    for (const id of descendantIds) checked.add(id);
    set({ checked3dIds: checked });

    // Attempt to load meshes for newly checked regions
    const toLoad = descendantIds.filter((id) => !get().loadedMeshes.has(id));
    if (toLoad.length > 0) {
      set({ meshesLoading: true, meshLoadProgress: { loaded: 0, total: toLoad.length } });
      let loaded = 0;
      for (let i = 0; i < toLoad.length; i += 6) {
        const batch = toLoad.slice(i, i + 6);
        await Promise.all(batch.map((id) => get().loadMesh(id).catch(() => null)));
        loaded += batch.length;
        set({ meshLoadProgress: { loaded: Math.min(loaded, toLoad.length), total: toLoad.length } });
      }
      set({ meshesLoading: false });
    }

    // Update visible: show only checked regions that have loaded meshes
    const allChecked = get().checked3dIds;
    const visible = new Set<number>();
    allChecked.forEach((id) => {
      if (get().loadedMeshes.has(id)) visible.add(id);
    });
    set({ visibleMeshIds: visible });
  },

  uncheck3dRegion: (structureId: number) => {
    const { ontology } = get();
    if (!ontology) return;

    // Synchronous import -- treeUtils is already statically imported by other modules
    // so dynamic import resolves instantly from cache
    import('../utils/treeUtils').then(({ findNode, collectDescendantIds }) => {
      const node = findNode(ontology, structureId);
      if (!node) return;

      const descendantIds = collectDescendantIds(node);
      const checked = new Set(get().checked3dIds);
      for (const id of descendantIds) checked.delete(id);
      set({ checked3dIds: checked });

      // Update visible
      if (checked.size === 0) {
        // If nothing checked, show all major regions
        const MAJOR = [997, 8, 567, 343, 512, 1009, 73, 315, 698, 1089, 549, 623, 803, 1097, 1065, 313, 771, 354];
        const visible = new Set<number>();
        for (const id of MAJOR) {
          if (get().loadedMeshes.has(id)) visible.add(id);
        }
        set({ visibleMeshIds: visible });
      } else {
        const visible = new Set<number>();
        checked.forEach((id) => {
          if (get().loadedMeshes.has(id)) visible.add(id);
        });
        set({ visibleMeshIds: visible });
      }
    });
  },

  clearAllChecked3d: () => {
    const MAJOR = [997, 8, 567, 343, 512, 1009, 73, 315, 698, 1089, 549, 623, 803, 1097, 1065, 313, 771, 354];
    const visible = new Set<number>();
    for (const id of MAJOR) {
      if (get().loadedMeshes.has(id)) visible.add(id);
    }
    set({ checked3dIds: new Set(), visibleMeshIds: visible });
  },

  clearFallback3d: () => set({ fallback3d: null }),
}));

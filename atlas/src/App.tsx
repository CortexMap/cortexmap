import { useEffect, Suspense, lazy } from 'react';
import { BrowserRouter, Routes, Route, useNavigate, useLocation } from 'react-router-dom';
import { useAtlasStore } from './store/atlasStore';
import { AppLayout } from './components/layout/AppLayout';
import { OntologyTree } from './components/tree/OntologyTree';
import { AtlasViewer } from './components/viewer/AtlasViewer';
import { RegionDetail } from './components/detail/RegionDetail';
import { SearchFunc } from './components/search/SearchFunc';
import './global.css';

// Lazy-load the 3D viewer so Three.js is only fetched when navigating to /3d
const BrainViewer3D = lazy(() =>
  import('./components/viewer3d/BrainViewer3D').then((m) => ({ default: m.BrainViewer3D }))
);
const ControlBar3D = lazy(() =>
  import('./components/viewer3d/BrainViewer3D').then((m) => ({ default: m.ControlBar }))
);

function Viewer3DFallback() {
  return (
    <div style={{
      display: 'flex', alignItems: 'center', justifyContent: 'center',
      height: '100%', background: '#fafaf9', color: '#6b7280',
      fontSize: 15,
    }}>
      Loading 3D viewer...
    </div>
  );
}

function ViewerWithNav() {
  const navigate = useNavigate();
  const location = useLocation();
  const is3D = location.pathname === '/3d';

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: 2,
        padding: '4px 8px',
        background: '#fafaf9',
        borderBottom: '1px solid #e5e5e4',
        flexShrink: 0,
      }}>
        <NavTab label="2D Atlas" active={!is3D} onClick={() => navigate('/')} />
        <NavTab label="3D Brain" active={is3D} onClick={() => navigate('/3d')} />
        <div style={{ flex: 1 }} />
        <SearchFunc />
      </div>
      <div style={{ flex: 1, overflow: 'hidden' }}>
        <Routes>
          <Route path="/" element={<AtlasViewer />} />
          <Route path="/3d" element={
            <Suspense fallback={<Viewer3DFallback />}>
              <BrainViewer3D />
            </Suspense>
          } />
        </Routes>
      </div>
    </div>
  );
}

function NavTab({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        padding: '4px 14px',
        border: 'none',
        borderRadius: 4,
        background: active ? '#7c3aed' : 'transparent',
        color: active ? '#fff' : '#374151',
        fontSize: 13,
        fontWeight: active ? 600 : 500,
        fontFamily: 'system-ui, -apple-system, sans-serif',
        cursor: 'pointer',
        transition: 'all 0.15s',
      }}
    >
      {label}
    </button>
  );
}

function AppContent() {
  const { loadOntology, loadSections } = useAtlasStore();
  const location = useLocation();

  useEffect(() => {
    loadOntology();
    loadSections();
  }, [loadOntology, loadSections]);

  const is3D = location.pathname === '/3d';

  return (
    <AppLayout
      left={<OntologyTree />}
      center={<ViewerWithNav />}
      right={<RegionDetail />}
      bottom={is3D ? (
        <Suspense fallback={null}>
          <ControlBar3D />
        </Suspense>
      ) : undefined}
    />
  );
}

export default function App() {
  return (
    <BrowserRouter>
      <AppContent />
    </BrowserRouter>
  );
}

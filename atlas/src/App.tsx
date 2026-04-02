import { useEffect } from 'react';
import { useAtlasStore } from './store/atlasStore';
import { AppLayout } from './components/layout/AppLayout';
import { OntologyTree } from './components/tree/OntologyTree';
import { AtlasViewer } from './components/viewer/AtlasViewer';
import { RegionDetail } from './components/detail/RegionDetail';
import './global.css';

export default function App() {
  const { loadOntology, loadSections } = useAtlasStore();

  useEffect(() => {
    loadOntology();
    loadSections();
  }, [loadOntology, loadSections]);

  return (
    <AppLayout
      left={<OntologyTree />}
      center={<AtlasViewer />}
      right={<RegionDetail />}
    />
  );
}

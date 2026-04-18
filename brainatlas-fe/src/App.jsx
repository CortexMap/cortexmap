import { useState, useEffect } from 'react';
import axios from 'axios';
import RegionList from './components/RegionList';
import RegionDetail from './components/RegionDetail';
import PipelineStats from './components/PipelineStats';
import WorkerManagement from './components/WorkerManagement';
import { Brain } from 'lucide-react';
import { API_BASE_URL, logger } from './config';
import './App.css';

// Read the ?region=<uuid> query param. Returns null if absent or not a UUID-like string.
const getRegionIdFromUrl = () => {
  const params = new URLSearchParams(window.location.search);
  const id = params.get('region');
  if (!id) return null;
  // Loose UUID check — don't bother validating strictly, server will reject bad ids
  return /^[0-9a-f-]{10,}$/i.test(id) ? id : null;
};

// Push/replace the ?region=<uuid> query param without reloading the page.
const setRegionIdInUrl = (regionId) => {
  const url = new URL(window.location.href);
  if (regionId) {
    url.searchParams.set('region', regionId);
  } else {
    url.searchParams.delete('region');
  }
  window.history.pushState({ regionId }, '', url.toString());
};

function App() {
  const [regions, setRegions] = useState([]);
  const [selectedRegion, setSelectedRegion] = useState(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);

  useEffect(() => {
    logger.info('App component mounted');
    logger.info('API Base URL:', API_BASE_URL);
    fetchRegions();
  }, []);

  // Listen to browser back/forward so the URL stays in sync with state.
  useEffect(() => {
    const handlePopState = () => {
      const regionId = getRegionIdFromUrl();
      if (!regionId) {
        setSelectedRegion(null);
        return;
      }
      // Try to find in already-loaded list; otherwise fetch minimally.
      const match = regions.find(r => r.id === regionId);
      if (match) {
        setSelectedRegion(match);
      } else if (regions.length > 0) {
        // Regions loaded but none matched — clear URL and fall back to list.
        logger.warn(`Region ${regionId} from URL not found in loaded regions`);
        setRegionIdInUrl(null);
        setSelectedRegion(null);
      }
    };
    window.addEventListener('popstate', handlePopState);
    return () => window.removeEventListener('popstate', handlePopState);
  }, [regions]);

  // Once regions are loaded, apply any ?region= param from the URL.
  useEffect(() => {
    if (regions.length === 0) return;
    const urlRegionId = getRegionIdFromUrl();
    if (!urlRegionId) return;
    if (selectedRegion?.id === urlRegionId) return;

    const match = regions.find(r => r.id === urlRegionId);
    if (match) {
      logger.info(`Auto-selecting region ${match.name} from URL param`);
      setSelectedRegion(match);
    } else {
      logger.warn(`Region ${urlRegionId} from URL not found in regions list`);
      setRegionIdInUrl(null);
    }
  }, [regions]);

  const fetchRegions = async () => {
    const url = `${API_BASE_URL}/regions`;
    logger.api('GET', url);
    
    try {
      setLoading(true);
      setError(null);
      
      logger.debug('Fetching regions from backend...');
      const response = await axios.get(url);
      
      logger.apiSuccess('GET', url, {
        status: response.status,
        dataLength: response.data?.length,
        firstRegion: response.data?.[0]
      });
      
      setRegions(response.data);
      logger.info(`Successfully loaded ${response.data.length} regions`);
    } catch (err) {
      logger.apiError('GET', url, err);
      
      let errorMessage = 'Failed to fetch regions. ';
      
      if (err.code === 'ERR_NETWORK' || err.message === 'Network Error') {
        errorMessage += `Cannot connect to backend at ${API_BASE_URL}. Please check if the server is running.`;
        logger.error('Network error - backend server may be offline or unreachable');
      } else if (err.response) {
        errorMessage += `Server responded with ${err.response.status}: ${err.response.statusText}`;
        logger.error('Server error response:', err.response.data);
      } else if (err.request) {
        errorMessage += 'No response received from server. Please check your network connection.';
        logger.error('Request made but no response received');
      } else {
        errorMessage += err.message;
        logger.error('Request setup error:', err.message);
      }
      
      setError(errorMessage);
    } finally {
      setLoading(false);
    }
  };

  const handleRegionSelect = (region) => {
    setSelectedRegion(region);
    setRegionIdInUrl(region.id);
  };

  const handleBackToList = () => {
    setSelectedRegion(null);
    setRegionIdInUrl(null);
  };

  return (
    <div className="app">
      <header className="app-header">
        <div className="header-content">
          <div className="logo-section">
            <Brain size={40} className="brain-icon" />
            <h1>Brain Atlas Explorer</h1>
          </div>
          <p className="subtitle">Explore brain regions with AI-powered summaries</p>
        </div>
      </header>

      <main className="app-main">
        <WorkerManagement />
        <PipelineStats />
        
        {error && (
          <div className="error-banner">
            <p>{error}</p>
          </div>
        )}

        {loading ? (
          <div className="loading-container">
            <div className="spinner"></div>
            <p>Loading brain regions...</p>
          </div>
        ) : selectedRegion ? (
          <RegionDetail 
            region={selectedRegion} 
            onBack={handleBackToList}
          />
        ) : (
          <RegionList 
            regions={regions} 
            onRegionSelect={handleRegionSelect}
          />
        )}
      </main>

      <footer className="app-footer">
        <p>Brain Atlas Explorer - AI-Powered Neuroscience Research Tool</p>
      </footer>
    </div>
  );
}

export default App;

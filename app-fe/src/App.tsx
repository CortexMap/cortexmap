import React, { useState, useEffect, useCallback } from 'react';
import SearchBar from './components/SearchBar';
import BrainRegionCard from './components/BrainRegionCard';
import { BrainRegion } from './types';
import { fetchBrainRegions } from './api/brainRegions';
import './App.css';

const DEBOUNCE_MS = 300;

function App() {
  const [searchQuery, setSearchQuery] = useState('');
  const [regions, setRegions] = useState<BrainRegion[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadRegions = useCallback(async (query: string) => {
    setLoading(true);
    setError(null);
    try {
      const data = await fetchBrainRegions(query);
      setRegions(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load brain regions');
      setRegions([]);
    } finally {
      setLoading(false);
    }
  }, []);

  // Load regions: initial load + debounced search when user types
  useEffect(() => {
    if (!searchQuery.trim()) {
      loadRegions('');
      return;
    }
    const timer = setTimeout(() => {
      loadRegions(searchQuery);
    }, DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [searchQuery, loadRegions]);

  return (
    <div className="app">
      <div className="app-container">
        <header className="app-header">
          <h1 className="app-title">CortexMap</h1>
          <p className="app-subtitle">
            Search brain regions by name, location, or function
          </p>
        </header>

        <SearchBar
          value={searchQuery}
          onChange={setSearchQuery}
          placeholder="Search brain regions..."
        />

        {error && (
          <div className="error-state" role="alert">
            <p>{error}</p>
            <p className="error-hint">
              Ensure the BFF server is running (cargo run -p cortexmap-bff) and
              the Python gRPC server is up on port 5005.
            </p>
          </div>
        )}

        {loading && (
          <div className="loading-state">
            <p>Loading brain regions…</p>
          </div>
        )}

        {!loading && !error && (
          <section className="card-list" aria-label="Brain region results">
            {regions.length > 0 ? (
              <ul className="card-list-inner">
                {regions.map((region) => (
                  <li key={region.id}>
                    <BrainRegionCard region={region} />
                  </li>
                ))}
              </ul>
            ) : (
              <div className="empty-state">
                <p>
                  {searchQuery.trim()
                    ? `No brain regions match "${searchQuery}". Try another term.`
                    : 'No data to show.'}
                </p>
              </div>
            )}
          </section>
        )}
      </div>
    </div>
  );
}

export default App;

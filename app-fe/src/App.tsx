import React, { useState, useMemo } from 'react';
import SearchBar from './components/SearchBar';
import BrainRegionCard from './components/BrainRegionCard';
import { BrainRegion } from './types';
import data from './data.json';
import './App.css';

const allRegions: BrainRegion[] = data as BrainRegion[];

function matchesSearch(region: BrainRegion, query: string): boolean {
  if (!query.trim()) return true;
  const q = query.trim().toLowerCase();
  const name = region.name.toLowerCase();
  const id = region.id.toLowerCase();
  const lobe = region.location.lobe.toLowerCase();
  const regionName = region.location.anatomical_region.toLowerCase();
  const func = region.function_diseases.function_description.toLowerCase();
  const diseases = region.function_diseases.disease_description.toLowerCase();
  return (
    name.includes(q) ||
    id.includes(q) ||
    lobe.includes(q) ||
    regionName.includes(q) ||
    func.includes(q) ||
    diseases.includes(q)
  );
}

const App: React.FC = () => {
  const [searchQuery, setSearchQuery] = useState('');

  const filteredRegions = useMemo(() => {
    return allRegions.filter((r) => matchesSearch(r, searchQuery));
  }, [searchQuery]);

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

        <section className="card-list" aria-label="Brain region results">
          {filteredRegions.length > 0 ? (
            <ul className="card-list-inner">
              {filteredRegions.map((region) => (
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
      </div>
    </div>
  );
};

export default App;

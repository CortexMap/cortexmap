import { useState, useMemo } from 'react';
import { Search, Filter } from 'lucide-react';
import './RegionList.css';

function RegionList({ regions, onRegionSelect }) {
  const [searchTerm, setSearchTerm] = useState('');
  const [sortBy, setSortBy] = useState('name');

  const filteredAndSortedRegions = useMemo(() => {
    let filtered = regions.filter(region =>
      region.name.toLowerCase().includes(searchTerm.toLowerCase()) ||
      region.acronym.toLowerCase().includes(searchTerm.toLowerCase())
    );

    return filtered.sort((a, b) => {
      if (sortBy === 'name') {
        return a.name.localeCompare(b.name);
      } else if (sortBy === 'acronym') {
        return a.acronym.localeCompare(b.acronym);
      } else if (sortBy === 'structure_order') {
        return a.structure_order - b.structure_order;
      }
      return 0;
    });
  }, [regions, searchTerm, sortBy]);

  const rgbToHex = (color) => {
    if (!color) return '#6366f1';
    const { red, green, blue } = color;
    return `#${((1 << 24) + (red << 16) + (green << 8) + blue).toString(16).slice(1)}`;
  };

  return (
    <div className="region-list-container">
      <div className="controls-bar">
        <div className="search-box">
          <Search size={20} className="search-icon" />
          <input
            type="text"
            placeholder="Search regions by name or acronym..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="search-input"
          />
        </div>
        
        <div className="filter-box">
          <Filter size={20} className="filter-icon" />
          <select
            value={sortBy}
            onChange={(e) => setSortBy(e.target.value)}
            className="sort-select"
          >
            <option value="name">Sort by Name</option>
            <option value="acronym">Sort by Acronym</option>
            <option value="structure_order">Sort by Structure Order</option>
          </select>
        </div>
      </div>

      <div className="region-stats">
        <p>Showing {filteredAndSortedRegions.length} of {regions.length} regions</p>
      </div>

      <div className="regions-grid">
        {filteredAndSortedRegions.map((region) => (
          <div
            key={region.id}
            className="region-card"
            onClick={() => onRegionSelect(region)}
          >
            <div
              className="region-color-bar"
              style={{ backgroundColor: rgbToHex(region.color) }}
            />
            <div className="region-card-content">
              <div className="region-header">
                <h3 className="region-name">{region.name}</h3>
                <span className="region-acronym">{region.acronym}</span>
              </div>
              
              <div className="region-metadata">
                <div className="metadata-item">
                  <span className="metadata-label">Region ID:</span>
                  <span className="metadata-value">{region.region_id}</span>
                </div>
                
                {region.parent_acronym && (
                  <div className="metadata-item">
                    <span className="metadata-label">Parent:</span>
                    <span className="metadata-value">{region.parent_acronym}</span>
                  </div>
                )}
                
                <div className="metadata-item">
                  <span className="metadata-label">Order:</span>
                  <span className="metadata-value">{region.structure_order}</span>
                </div>
              </div>
            </div>
          </div>
        ))}
      </div>

      {filteredAndSortedRegions.length === 0 && (
        <div className="no-results">
          <p>No regions found matching "{searchTerm}"</p>
        </div>
      )}
    </div>
  );
}

export default RegionList;

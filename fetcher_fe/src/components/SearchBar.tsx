import React, { useState } from 'react';
import './SearchBar.css';

interface SearchBarProps {
  onSearch: (query: string, pageSize: number) => void;
  isSearching: boolean;
}

const SearchBar: React.FC<SearchBarProps> = ({ onSearch, isSearching }) => {
  const [query, setQuery] = useState('');
  const [pageSize, setPageSize] = useState(10);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (query.trim() && !isSearching) {
      onSearch(query.trim(), pageSize);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="search-bar">
      <input
        type="text"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Enter PubMed search query..."
        className="search-input"
        disabled={isSearching}
      />
      <div className="page-size-wrapper">
        <label htmlFor="page-size" className="page-size-label">Papers:</label>
        <input
          id="page-size"
          type="number"
          value={pageSize}
          onChange={(e) => setPageSize(Math.max(1, Math.min(20, parseInt(e.target.value) || 10)))}
          min="1"
          max="20"
          className="page-size-input"
          disabled={isSearching}
          title="Total papers to fetch (1-20)"
        />
      </div>
      <button 
        type="submit" 
        className="search-button"
        disabled={isSearching || !query.trim()}
      >
        {isSearching ? 'Searching...' : 'Search'}
      </button>
    </form>
  );
};

export default SearchBar;

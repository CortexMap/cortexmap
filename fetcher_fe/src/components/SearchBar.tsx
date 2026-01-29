import React, { useState } from 'react';
import './SearchBar.css';

interface SearchBarProps {
  onSearch: (query: string) => void;
  isSearching: boolean;
}

const SearchBar: React.FC<SearchBarProps> = ({ onSearch, isSearching }) => {
  const [query, setQuery] = useState('');

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (query.trim() && !isSearching) {
      onSearch(query.trim());
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

import React, { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import SearchBar from '../components/SearchBar';
import WorkersSection from '../components/WorkersSection';
import './HomePage.css';

const HomePage: React.FC = () => {
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);

  const handleSearch = async (query: string, pageSize: number) => {
    setError(null);
    navigate('/query', { state: { query, pageSize } });
  };

  return (
    <div className="home-page">
      <header className="page-header">
        <h1>CortexMap Paper Fetcher</h1>
        <p>Search PubMed papers and track real-time fetching status</p>
        <button onClick={() => navigate('/history')} className="history-link">
          📋 View History
        </button>
      </header>

      <SearchBar onSearch={handleSearch} isSearching={false} />

      {error && <div className="error-banner">{error}</div>}

      <WorkersSection />
    </div>
  );
};

export default HomePage;

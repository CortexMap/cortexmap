import React from 'react';
import SearchBar from './components/SearchBar';
import PaperList from './components/PaperList';
import { usePaperFetcher } from './hooks/usePaperFetcher';
import './App.css';

const App: React.FC = () => {
  const { papers, isSearching, search, retryQueueLength } = usePaperFetcher();

  return (
    <div className="app">
      <div className="app-container">
        <header className="app-header">
          <h1 className="app-title">PubMed Paper Fetcher</h1>
          <p className="app-subtitle">
            Search for papers and track real-time fetching status
          </p>
        </header>

        <SearchBar onSearch={search} isSearching={isSearching} />

        {isSearching && (
          <div className="loading-message">
            <div className="loading-spinner"></div>
            <span>Searching PubMed...</span>
          </div>
        )}

        <PaperList papers={papers} retryQueueLength={retryQueueLength} />

        {!isSearching && papers.length === 0 && (
          <div className="empty-state">
            <div className="empty-state-icon">🔬</div>
            <h3>Start Your Search</h3>
            <p>Enter a query above to search for PubMed papers</p>
            <div className="features">
              <div className="feature">
                <span className="feature-icon">⚡</span>
                <span>Real-time status updates</span>
              </div>
              <div className="feature">
                <span className="feature-icon">🔄</span>
                <span>Automatic retry on failures</span>
              </div>
              <div className="feature">
                <span className="feature-icon">📊</span>
                <span>Track metadata, abstract, and PDF</span>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default App;

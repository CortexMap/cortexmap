import React, { useState } from 'react';
import SearchBar from './components/SearchBar';
import PaperList from './components/PaperList';
import { usePaperFetcher } from './hooks/usePaperFetcher';
import './App.css';

const App: React.FC = () => {
  const { papers, isSearching, search, queueStats, allocateWorkers, stopWorkers } = usePaperFetcher();
  const [workerCount, setWorkerCount] = useState(2);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const handleSearch = async (query: string) => {
    setErrorMessage(null);
    try {
      await search(query);
    } catch (error) {
      setErrorMessage((error as Error).message);
    }
  };

  const handleAllocateWorkers = async () => {
    try {
      await allocateWorkers(workerCount);
    } catch (error) {
      setErrorMessage((error as Error).message);
    }
  };

  const handleStopWorkers = async () => {
    try {
      await stopWorkers();
    } catch (error) {
      setErrorMessage((error as Error).message);
    }
  };

  return (
    <div className="app">
      <div className="app-container">
        <header className="app-header">
          <h1 className="app-title">CortexMap Paper Fetcher</h1>
          <p className="app-subtitle">
            Search PubMed papers and track real-time fetching status
          </p>
        </header>

        <div className="controls-section">
          <SearchBar onSearch={handleSearch} isSearching={isSearching} />
          
          <div className="worker-controls">
            <div className="worker-input-group">
              <label htmlFor="worker-count">Workers:</label>
              <input
                id="worker-count"
                type="number"
                min="1"
                max="10"
                value={workerCount}
                onChange={(e) => setWorkerCount(parseInt(e.target.value) || 1)}
                className="worker-input"
              />
            </div>
            <button onClick={handleAllocateWorkers} className="worker-button allocate">
              Start Workers
            </button>
            <button onClick={handleStopWorkers} className="worker-button stop">
              Stop Workers
            </button>
          </div>
        </div>

        {errorMessage && (
          <div className="error-banner">
            <span className="error-icon">⚠️</span>
            <span>{errorMessage}</span>
          </div>
        )}

        {isSearching && (
          <div className="loading-message">
            <div className="loading-spinner"></div>
            <span>Enqueueing papers...</span>
          </div>
        )}

        <PaperList papers={papers} queueStats={queueStats} />

        {!isSearching && papers.length === 0 && (
          <div className="empty-state">
            <div className="empty-state-icon">🔬</div>
            <h3>Start Your Search</h3>
            <p>Enter a PubMed query above and allocate workers to begin fetching</p>
            <div className="features">
              <div className="feature">
                <span className="feature-icon">⚡</span>
                <span>Real-time status updates (200ms polling)</span>
              </div>
              <div className="feature">
                <span className="feature-icon">🔄</span>
                <span>Automatic retry on failures (max 3 attempts)</span>
              </div>
              <div className="feature">
                <span className="feature-icon">📊</span>
                <span>Track summary, abstract, and PDF components</span>
              </div>
              <div className="feature">
                <span className="feature-icon">☁️</span>
                <span>S3 storage for fetched content</span>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default App;

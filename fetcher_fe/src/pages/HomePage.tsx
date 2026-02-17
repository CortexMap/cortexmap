import React, { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import SearchBar from '../components/SearchBar';
import WorkersSection from '../components/WorkersSection';
import { api } from '../api/backendApi';
import { RecentTask } from '../types';
import './HomePage.css';

const HomePage: React.FC = () => {
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);
  const [recentTasks, setRecentTasks] = useState<RecentTask[]>([]);

  useEffect(() => {
    const fetchRecentTasks = async () => {
      try {
        const stats = await api.getQueueStatus();
        setRecentTasks(stats.recentTasks || []);
      } catch (err) {
        console.error('Failed to fetch recent tasks:', err);
      }
    };

    fetchRecentTasks();
    const interval = setInterval(fetchRecentTasks, 3000);
    return () => clearInterval(interval);
  }, []);

  const handleSearch = async (query: string, pageSize: number) => {
    setError(null);
    navigate('/query', { state: { query, pageSize } });
  };

  const inProgressTasks = recentTasks.filter(t => t.status === 'in_progress' || t.status === 'pending');

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

      {inProgressTasks.length > 0 && (
        <div className="recent-papers">
          <h3>Recent / In-Progress Papers</h3>
          <div className="recent-papers-list">
            {inProgressTasks.map((task) => (
              <div key={task.pmcId} className="recent-paper-card">
                <div className="paper-header">
                  <span className="pmc-id">{task.pmcId}</span>
                  <span className={`status-badge ${task.status}`}>{task.status}</span>
                </div>
                <div className="paper-progress">
                  <div className="progress-bar">
                    <div 
                      className="progress-fill" 
                      style={{ width: `${(task.componentsCompleted / task.totalComponents) * 100}%` }}
                    />
                  </div>
                  <span className="progress-text">
                    {task.componentsCompleted}/{task.totalComponents} components
                  </span>
                </div>
                {task.workerId && <div className="worker-id">Worker: {task.workerId}</div>}
              </div>
            ))}
          </div>
        </div>
      )}

      <WorkersSection />
    </div>
  );
};

export default HomePage;

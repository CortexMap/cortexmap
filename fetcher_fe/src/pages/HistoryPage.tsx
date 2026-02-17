import React, { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { queryHistoryService } from '../services/queryHistory';
import { QueryHistory } from '../types/enhanced';
import './HistoryPage.css';

const HistoryPage: React.FC = () => {
  const navigate = useNavigate();
  const [history, setHistory] = useState<QueryHistory[]>([]);

  useEffect(() => {
    setHistory(queryHistoryService.getAll());
  }, []);

  const handleQueryClick = (query: QueryHistory) => {
    navigate('/query', { state: { query: query.query, historyId: query.id } });
  };

  const handleDelete = (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    queryHistoryService.delete(id);
    setHistory(queryHistoryService.getAll());
  };

  const handleClearAll = () => {
    if (confirm('Clear all query history?')) {
      queryHistoryService.clear();
      setHistory([]);
    }
  };

  return (
    <div className="history-page">
      <div className="history-header">
        <button onClick={() => navigate('/')} className="back-btn">← Back</button>
        <h1>Query History</h1>
        {history.length > 0 && (
          <button onClick={handleClearAll} className="clear-btn">Clear All</button>
        )}
      </div>

      {history.length === 0 ? (
        <div className="empty-history">
          <div className="empty-icon">📋</div>
          <h3>No queries yet</h3>
          <p>Your search history will appear here</p>
          <button onClick={() => navigate('/')} className="primary-btn">Start Searching</button>
        </div>
      ) : (
        <div className="history-list">
          {history.map((item) => (
            <div key={item.id} className="history-item" onClick={() => handleQueryClick(item)}>
              <div className="history-content">
                <div className="query-text">"{item.query}"</div>
                <div className="history-meta">
                  <span className="timestamp">{new Date(item.timestamp).toLocaleString()}</span>
                  <span className="papers-count">{item.tasksEnqueued} papers</span>
                  <span className={`status-badge ${item.status}`}>{item.status}</span>
                </div>
                {item.pmcIds.length > 0 && (
                  <div className="pmc-preview">
                    {item.pmcIds.slice(0, 3).map(id => (
                      <span key={id} className="pmc-chip">{id}</span>
                    ))}
                    {item.pmcIds.length > 3 && (
                      <span className="more-count">+{item.pmcIds.length - 3} more</span>
                    )}
                  </div>
                )}
              </div>
              <button onClick={(e) => handleDelete(item.id, e)} className="delete-btn">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <path d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2M10 11v6M14 11v6"/>
                </svg>
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

export default HistoryPage;

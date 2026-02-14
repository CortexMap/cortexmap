import React, { useState, useEffect } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import PaperList from '../components/PaperList';
import WorkersSection from '../components/WorkersSection';
import { usePaperFetcher } from '../hooks/usePaperFetcher';
import { queryHistoryService } from '../services/queryHistory';
import './QueryPage.css';

interface QueryDetails {
  query: string;
  tasksEnqueued: number;
  pmcIds: string[];
  timestamp: number;
}

const QueryPage: React.FC = () => {
  const location = useLocation();
  const navigate = useNavigate();
  const { papers, isSearching, search, lastEnqueueResponse } = usePaperFetcher();
  const [queryDetails, setQueryDetails] = useState<QueryDetails | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [historyId, setHistoryId] = useState<string | null>(null);

  useEffect(() => {
    const query = location.state?.query;
    const pageSize = location.state?.pageSize || 3;
    const existingHistoryId = location.state?.historyId;
    
    if (!query) {
      navigate('/');
      return;
    }

    if (existingHistoryId) {
      setHistoryId(existingHistoryId);
    } else {
      // Create new history entry
      const newId = `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
      setHistoryId(newId);
      queryHistoryService.add({
        id: newId,
        query,
        timestamp: Date.now(),
        tasksEnqueued: 0,
        pmcIds: [],
        status: 'pending'
      });
    }

    const fetchQuery = async () => {
      try {
        await search(query, pageSize);
      } catch (err) {
        setError((err as Error).message);
      }
    };

    fetchQuery();
  }, [location.state]);

  // Update query details when search completes
  useEffect(() => {
    if (lastEnqueueResponse && location.state?.query) {
      const details = {
        query: location.state.query,
        tasksEnqueued: lastEnqueueResponse.tasksEnqueued,
        pmcIds: lastEnqueueResponse.pmcIds || [],
        timestamp: Date.now()
      };
      setQueryDetails(details);
      
      // Update history
      if (historyId) {
        queryHistoryService.update(historyId, {
          tasksEnqueued: details.tasksEnqueued,
          pmcIds: details.pmcIds,
          status: 'in_progress'
        });
      }
    }
  }, [lastEnqueueResponse, location.state, historyId]);

  // Update status when papers complete
  useEffect(() => {
    if (papers.length > 0 && historyId) {
      const allCompleted = papers.every(p => p.status === 'completed');
      const anyFailed = papers.some(p => p.status === 'failed');
      
      if (allCompleted) {
        queryHistoryService.update(historyId, { status: 'completed' });
      } else if (anyFailed) {
        queryHistoryService.update(historyId, { status: 'failed' });
      }
    }
  }, [papers, historyId]);

  if (!queryDetails) {
    return (
      <div className="query-page">
        <div className="loading">Loading query...</div>
      </div>
    );
  }

  return (
    <div className="query-page">
      <button onClick={() => navigate('/')} className="back-btn">← Back</button>

      <div className="query-banner">
        <div className="query-header">
          <h2>Query Results</h2>
          <span className="query-time">
            {new Date(queryDetails.timestamp).toLocaleString()}
          </span>
        </div>
        
        <div className="query-info">
          <div className="query-text">
            <span className="label">Search Query:</span>
            <span className="value">"{queryDetails.query}"</span>
          </div>
          
          <div className="query-stats">
            <div className="stat">
              <span className="stat-value">{queryDetails.tasksEnqueued}</span>
              <span className="stat-label">Papers Enqueued</span>
            </div>
            <div className="stat">
              <span className="stat-value">{queryDetails.pmcIds.length}</span>
              <span className="stat-label">PMC IDs Found</span>
            </div>
          </div>
        </div>

        {queryDetails.pmcIds.length > 0 && (
          <div className="pmc-ids">
            <span className="label">PMC IDs:</span>
            <div className="ids-list">
              {queryDetails.pmcIds.map(id => (
                <span key={id} className="pmc-id">{id}</span>
              ))}
            </div>
          </div>
        )}
      </div>

      {error && <div className="error-message">{error}</div>}

      {queryDetails.tasksEnqueued === 0 && (
        <div className="warning-message">
          ⚠️ No papers were enqueued. This might be due to:
          <ul>
            <li>PubMed API rate limiting (429 error) - try again in a few seconds</li>
            <li>No results found for this query</li>
            <li>Query term too generic or misspelled</li>
          </ul>
        </div>
      )}

      <WorkersSection />

      {isSearching && (
        <div className="loading-message">
          <div className="spinner"></div>
          <span>Fetching paper details...</span>
        </div>
      )}

      {queryDetails.pmcIds.length === 0 ? (
        <div className="no-results">
          <h3>No papers found</h3>
          <p>Try a different search query</p>
        </div>
      ) : (
        <PaperList papers={papers} />
      )}
    </div>
  );
};

export default QueryPage;

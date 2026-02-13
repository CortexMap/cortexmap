import React from 'react';
import { PaperState, QueueStats } from '../types';
import PaperCard from './PaperCard';
import './PaperList.css';

interface PaperListProps {
  papers: PaperState[];
  queueStats: QueueStats | null;
}

const PaperList: React.FC<PaperListProps> = ({ papers, queueStats }) => {
  if (papers.length === 0) {
    return null;
  }

  return (
    <div className="paper-list-container">
      <div className="paper-list-header">
        <h2>Found {papers.length} papers</h2>
        {queueStats && (
          <div className="queue-stats">
            <div className="stat">
              <span className="stat-label">Pending:</span>
              <span className="stat-value">{queueStats.pendingTasks}</span>
            </div>
            <div className="stat">
              <span className="stat-label">In Progress:</span>
              <span className="stat-value">{queueStats.inProgressTasks}</span>
            </div>
            <div className="stat">
              <span className="stat-label">Completed:</span>
              <span className="stat-value success">{queueStats.completedTasks}</span>
            </div>
            <div className="stat">
              <span className="stat-label">Failed:</span>
              <span className="stat-value failed">{queueStats.failedTasks}</span>
            </div>
            <div className="stat">
              <span className="stat-label">Workers:</span>
              <span className="stat-value workers">{queueStats.activeWorkers}</span>
            </div>
          </div>
        )}
      </div>
      <div className="paper-list">
        {papers.map((paperState) => (
          <PaperCard key={paperState.pmcId} paperState={paperState} />
        ))}
      </div>
    </div>
  );
};

export default PaperList;

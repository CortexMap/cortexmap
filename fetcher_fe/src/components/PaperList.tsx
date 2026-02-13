import React from 'react';
import { PaperFetchState } from '../types';
import PaperCard from './PaperCard';
import './PaperList.css';

interface PaperListProps {
  papers: PaperFetchState[];
  retryQueueLength: number;
}

const PaperList: React.FC<PaperListProps> = ({ papers, retryQueueLength }) => {
  if (papers.length === 0) {
    return null;
  }

  return (
    <div className="paper-list-container">
      <div className="paper-list-header">
        <h2>Found {papers.length} papers</h2>
        {retryQueueLength > 0 && (
          <div className="retry-queue-info">
            <span className="retry-icon">🔄</span>
            <span>{retryQueueLength} items in retry queue</span>
          </div>
        )}
      </div>
      <div className="paper-list">
        {papers.map((paperState) => (
          <PaperCard key={paperState.paper.id} paperState={paperState} />
        ))}
      </div>
    </div>
  );
};

export default PaperList;

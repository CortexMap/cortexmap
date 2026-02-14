import React from 'react';
import { PaperState } from '../types';
import PaperCard from './PaperCard';
import './PaperList.css';

interface PaperListProps {
  papers: PaperState[];
}

const PaperList: React.FC<PaperListProps> = ({ papers }) => {
  if (papers.length === 0) {
    return null;
  }

  return (
    <div className="paper-list-container">
      <div className="paper-list-header">
        <h2>Found {papers.length} papers</h2>
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

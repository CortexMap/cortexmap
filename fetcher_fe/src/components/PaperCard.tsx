import React, { useState } from 'react';
import ReactMarkdown from 'react-markdown';
import { PaperState } from '../types';
import './PaperCard.css';

interface PaperCardProps {
  paperState: PaperState;
}

const PaperCard: React.FC<PaperCardProps> = ({ paperState }) => {
  const { pmcId, status, summary, abstract } = paperState;
  const [expandedSummary, setExpandedSummary] = useState(false);
  const [expandedAbstract, setExpandedAbstract] = useState(false);

  const renderSection = (title: string, icon: string, content: string, expanded: boolean, setExpanded: (val: boolean) => void) => (
    <div className="content-section">
      <div className="content-header" onClick={() => setExpanded(!expanded)}>
        <strong>{icon} {title}</strong>
        <span className="expand-icon">{expanded ? '▼' : '▶'}</span>
      </div>
      {expanded && (
        <div className="content-text markdown-content">
          <ReactMarkdown>{content}</ReactMarkdown>
        </div>
      )}
    </div>
  );

  return (
    <div className="paper-card">
      <div className="paper-header">
        <div className="paper-id">
          <span className="paper-id-label">PMC ID:</span>
          <span className="paper-id-value">{pmcId}</span>
        </div>
        <div className={`paper-status status-${status}`}>
          <span className="status-value">{status.replace('_', ' ').toUpperCase()}</span>
        </div>
      </div>

      {(summary || abstract) && (
        <div className="paper-content">
          {summary && renderSection('Summary', '📄', summary, expandedSummary, setExpandedSummary)}
          {abstract && renderSection('Abstract', '📝', abstract, expandedAbstract, setExpandedAbstract)}
        </div>
      )}
    </div>
  );
};

export default PaperCard;

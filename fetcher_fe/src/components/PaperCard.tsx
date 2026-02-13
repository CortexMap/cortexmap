import React from 'react';
import { PaperState, ComponentType } from '../types';
import StatusIndicator from './StatusIndicator';
import './PaperCard.css';

interface PaperCardProps {
  paperState: PaperState;
}

const PaperCard: React.FC<PaperCardProps> = ({ paperState }) => {
  const { pmcId, status, components } = paperState;

  const getComponent = (type: ComponentType) => {
    return components.get(type) || {
      componentType: type,
      status: 'pending' as const,
      attemptCount: 0,
      maxAttempts: 3,
    };
  };

  const summary = getComponent('summary');
  const abstract = getComponent('abstract');
  const pdf = getComponent('pdf');

  return (
    <div className="paper-card">
      <div className="paper-header">
        <div className="paper-id">
          <span className="paper-id-label">PMC ID:</span>
          <span className="paper-id-value">{pmcId}</span>
        </div>
        <div className={`paper-status status-${status}`}>
          <span className="status-label">Status:</span>
          <span className="status-value">{status.replace('_', ' ').toUpperCase()}</span>
        </div>
      </div>

      <div className="components-status">
        <div className="component-row">
          <StatusIndicator 
            status={summary.status} 
            attemptCount={summary.attemptCount} 
            maxAttempts={summary.maxAttempts}
          />
          <span className="component-label">Summary</span>
          {summary.status === 'completed' && summary.s3Key && (
            <div className="component-details">
              <span className="s3-key">{summary.s3Key}</span>
            </div>
          )}
          {summary.errorMessage && (
            <span className="error-message">{summary.errorMessage}</span>
          )}
        </div>

        <div className="component-row">
          <StatusIndicator 
            status={abstract.status} 
            attemptCount={abstract.attemptCount} 
            maxAttempts={abstract.maxAttempts}
          />
          <span className="component-label">Abstract</span>
          {abstract.status === 'completed' && abstract.s3Key && (
            <div className="component-details">
              <span className="s3-key">{abstract.s3Key}</span>
            </div>
          )}
          {abstract.errorMessage && (
            <span className="error-message">{abstract.errorMessage}</span>
          )}
        </div>

        <div className="component-row">
          <StatusIndicator 
            status={pdf.status} 
            attemptCount={pdf.attemptCount} 
            maxAttempts={pdf.maxAttempts}
          />
          <span className="component-label">PDF</span>
          {pdf.status === 'completed' && pdf.s3Key && (
            <div className="component-details">
              <span className="s3-key">{pdf.s3Key}</span>
            </div>
          )}
          {pdf.errorMessage && (
            <span className="error-message">{pdf.errorMessage}</span>
          )}
        </div>
      </div>
    </div>
  );
};

export default PaperCard;

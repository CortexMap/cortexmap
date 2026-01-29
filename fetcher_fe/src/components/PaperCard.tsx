import React from 'react';
import { PaperFetchState, FetchStatus } from '../types';
import StatusIndicator from './StatusIndicator';
import './PaperCard.css';

interface PaperCardProps {
  paperState: PaperFetchState;
}

const PaperCard: React.FC<PaperCardProps> = ({ paperState }) => {
  const { paper, components } = paperState;

  return (
    <div className="paper-card">
      <div className="paper-header">
        <div className="paper-id">
          <span className="paper-id-label">Paper ID:</span>
          <span className="paper-id-value">{paper.id}</span>
        </div>
        {paper.pmid && (
          <div className="paper-pmid">
            <span className="pmid-label">PMID:</span>
            <span className="pmid-value">{paper.pmid}</span>
          </div>
        )}
      </div>

      <div className="components-status">
        <div className="component-row">
          <StatusIndicator status={components.metadata.status} retryCount={components.metadata.retryCount} />
          <span className="component-label">Metadata</span>
          {components.metadata.status === FetchStatus.SUCCESS && paper.metadata && (
            <div className="component-details">
              <div className="metadata-preview">
                <strong>{paper.metadata.title}</strong>
                <div className="metadata-info">
                  <span>{paper.metadata.authors.join(', ')}</span>
                  <span>{paper.metadata.journal}</span>
                  <span>{paper.metadata.publicationDate}</span>
                </div>
              </div>
            </div>
          )}
          {components.metadata.error && (
            <span className="error-message">{components.metadata.error}</span>
          )}
        </div>

        <div className="component-row">
          <StatusIndicator status={components.abstract.status} retryCount={components.abstract.retryCount} />
          <span className="component-label">Abstract</span>
          {components.abstract.status === FetchStatus.SUCCESS && paper.abstract && (
            <div className="component-details">
              <p className="abstract-preview">{paper.abstract.substring(0, 150)}...</p>
            </div>
          )}
          {components.abstract.error && (
            <span className="error-message">{components.abstract.error}</span>
          )}
        </div>

        <div className="component-row">
          <StatusIndicator status={components.pdf.status} retryCount={components.pdf.retryCount} />
          <span className="component-label">PDF</span>
          {components.pdf.status === FetchStatus.SUCCESS && paper.pdfUrl && (
            <div className="component-details">
              <a href={paper.pdfUrl} target="_blank" rel="noopener noreferrer" className="pdf-link">
                View PDF
              </a>
            </div>
          )}
          {components.pdf.error && (
            <span className="error-message">{components.pdf.error}</span>
          )}
        </div>
      </div>
    </div>
  );
};

export default PaperCard;

import React, { useState } from 'react';
import { BrainRegion } from '../types';
import './BrainRegionCard.css';

interface BrainRegionCardProps {
  region: BrainRegion;
}

const BrainRegionCard: React.FC<BrainRegionCardProps> = ({ region }) => {
  const { name, id, location, function_diseases } = region;
  const [expandedFunction, setExpandedFunction] = useState(false);
  const [expandedDisease, setExpandedDisease] = useState(false);

  const PREVIEW_LENGTH = 300;
  const functionText = function_diseases.function_description;
  const diseaseText = function_diseases.disease_description;
  
  const shouldTruncateFunction = functionText.length > PREVIEW_LENGTH;
  const shouldTruncateDisease = diseaseText.length > PREVIEW_LENGTH;

  const displayFunction = expandedFunction || !shouldTruncateFunction
    ? functionText
    : functionText.slice(0, PREVIEW_LENGTH) + '...';

  const displayDisease = expandedDisease || !shouldTruncateDisease
    ? diseaseText
    : diseaseText.slice(0, PREVIEW_LENGTH) + '...';

  return (
    <article className="brain-region-card">
      <header className="card-header">
        <h3 className="card-title">{name}</h3>
        <span className="card-id">{id}</span>
      </header>
      <div className="card-location">
        <span className="location-label">Location</span>
        <ul className="location-list">
          <li><strong>Hemisphere:</strong> {location.hemisphere}</li>
          <li><strong>Lobe:</strong> {location.lobe}</li>
          <li><strong>Region:</strong> {location.anatomical_region}</li>
        </ul>
      </div>
      <div className="card-section">
        <span className="section-label">Function</span>
        <p className="card-text">{displayFunction}</p>
        {shouldTruncateFunction && (
          <button
            className="read-more-btn"
            onClick={() => setExpandedFunction(!expandedFunction)}
          >
            {expandedFunction ? 'Read less' : 'Read more'}
          </button>
        )}
      </div>
      <div className="card-section">
        <span className="section-label">Related diseases</span>
        <p className="card-text card-diseases">{displayDisease}</p>
        {shouldTruncateDisease && (
          <button
            className="read-more-btn"
            onClick={() => setExpandedDisease(!expandedDisease)}
          >
            {expandedDisease ? 'Read less' : 'Read more'}
          </button>
        )}
      </div>
    </article>
  );
};

export default BrainRegionCard;

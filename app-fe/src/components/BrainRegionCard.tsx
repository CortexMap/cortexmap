import React from 'react';
import { BrainRegion } from '../types';
import './BrainRegionCard.css';

interface BrainRegionCardProps {
  region: BrainRegion;
}

const BrainRegionCard: React.FC<BrainRegionCardProps> = ({ region }) => {
  const { name, id, location, function_diseases } = region;
  const funcPreview = function_diseases.function_description.slice(0, 200);
  const diseasePreview = function_diseases.disease_description.slice(0, 150);

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
        <p className="card-text">{funcPreview}…</p>
      </div>
      <div className="card-section">
        <span className="section-label">Related diseases</span>
        <p className="card-text card-diseases">{diseasePreview}…</p>
      </div>
    </article>
  );
};

export default BrainRegionCard;

import { useState, useEffect } from 'react';
import axios from 'axios';
import { ArrowLeft, RefreshCw, Loader2, CheckCircle, AlertCircle } from 'lucide-react';
import { API_BASE_URL, logger } from '../config';
import SummaryDisplay from './SummaryDisplay';
import './RegionDetail.css';

function RegionDetail({ region, onBack }) {
  const [summaries, setSummaries] = useState([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(null);
  const [isGenerating, setIsGenerating] = useState(false);
  const [generatingMessage, setGeneratingMessage] = useState('');
  const [status, setStatus] = useState(null);

  useEffect(() => {
    logger.info('RegionDetail mounted for region:', region.name, region.id);
    fetchRegionStatus();
    fetchRegionSummaries();
  }, [region.id]);

  const fetchRegionStatus = async () => {
    const url = `${API_BASE_URL}/regions/${region.id}/status`;
    logger.api('GET', url);

    try {
      const response = await axios.get(url);
      logger.apiSuccess('GET', url, response.data);
      setStatus(response.data);
    } catch (err) {
      logger.apiError('GET', url, err);
    }
  };

  const fetchRegionSummaries = async () => {
    const url = `${API_BASE_URL}/regions/${region.id}/summaries`;
    logger.api('GET', url);

    try {
      const response = await axios.get(url);
      logger.apiSuccess('GET', url, {
        summaryCount: response.data?.summaries?.length || 0
      });
      const summariesData = response.data?.summaries || [];
      setSummaries(summariesData);
      logger.info(`Loaded ${summariesData.length} summaries for region ${region.name}`);
      setError(null);
    } catch (err) {
      logger.apiError('GET', url, err);
      setSummaries([]);
    }
  };

  const handleGenerateSummary = async () => {
    const url = `${API_BASE_URL}/regions/${region.id}/generate`;
    logger.api('POST', url);

    try {
      setIsGenerating(true);
      setGeneratingMessage('Generating summary from knowledge base...');
      setError(null);

      const response = await axios.post(url);
      logger.apiSuccess('POST', url, response.data);

      // Summary comes back directly in the response (synchronous)
      logger.info('Summary generated successfully:', response.data.summary_id);

      // Refresh summaries and status
      await fetchRegionSummaries();
      await fetchRegionStatus();

      setIsGenerating(false);
      setGeneratingMessage('');
    } catch (err) {
      logger.apiError('POST', url, err);
      const errorMsg = err.response?.data?.error || 'Failed to generate summary';
      setError(errorMsg);
      setIsGenerating(false);
      setGeneratingMessage('');
    }
  };

  const formatDate = (dateString) => {
    if (!dateString) return 'N/A';
    return new Date(dateString).toLocaleString();
  };

  const rgbToHex = (color) => {
    if (!color) return '#6366f1';
    const { red, green, blue } = color;
    return `#${((1 << 24) + (red << 16) + (green << 8) + blue).toString(16).slice(1)}`;
  };

  return (
    <div className="region-detail-container">
      <button className="back-button" onClick={onBack}>
        <ArrowLeft size={20} />
        Back to Regions
      </button>

      <div className="region-detail-header">
        <div
          className="region-color-indicator"
          style={{ backgroundColor: rgbToHex(region.color) }}
        />
        <div className="region-title-section">
          <h2 className="region-detail-name">{region.name}</h2>
          <span className="region-detail-acronym">{region.acronym}</span>
        </div>
      </div>

      {error && (
        <div className="error-message">
          <AlertCircle size={20} />
          <span>{error}</span>
        </div>
      )}

      <div className="region-info-grid">
        <div className="info-card">
          <h3>Region Information</h3>
          <div className="info-rows">
            <div className="info-row">
              <span className="info-label">Region ID:</span>
              <span className="info-value">{region.region_id}</span>
            </div>
            <div className="info-row">
              <span className="info-label">UUID:</span>
              <span className="info-value info-uuid">{region.id}</span>
            </div>
            {region.parent_acronym && (
              <div className="info-row">
                <span className="info-label">Parent Region:</span>
                <span className="info-value">{region.parent_acronym}</span>
              </div>
            )}
            <div className="info-row">
              <span className="info-label">Structure Order:</span>
              <span className="info-value">{region.structure_order}</span>
            </div>
          </div>
        </div>

        <div className="info-card status-card">
          <h3>Knowledge Base Status</h3>
          {status ? (
            <div className="status-content">
              <div className="info-rows">
                <div className="info-row">
                  <span className="info-label">Summary Count:</span>
                  <span className="info-value">{status.summary_count || 0}</span>
                </div>
                <div className="info-row">
                  <span className="info-label">Last Summary:</span>
                  <span className="info-value">{formatDate(status.last_summary_at)}</span>
                </div>
              </div>
            </div>
          ) : (
            <div className="status-loading">
              <Loader2 size={24} className="spinning" />
              <p>Loading status...</p>
            </div>
          )}
        </div>
      </div>

      <div className="summaries-section">
        <div className="summaries-header">
          <h3>Research Summaries</h3>
          <div className="summary-actions">
            <button
              className="action-button secondary"
              onClick={handleGenerateSummary}
              disabled={isGenerating}
            >
              {isGenerating ? (
                <>
                  <Loader2 size={18} className="spinning" />
                  Generating...
                </>
              ) : (
                <>
                  <RefreshCw size={18} />
                  Generate Summary
                </>
              )}
            </button>
          </div>
        </div>

        {isGenerating && generatingMessage && (
          <div className="generating-indicator">
            <Loader2 size={24} className="spinning" />
            <p>{generatingMessage}</p>
            <p className="generating-subtext">This may take 10-30 seconds while the AI analyzes the knowledge base.</p>
          </div>
        )}

        {summaries.length > 0 ? (
          <SummaryDisplay summaries={summaries} formatDate={formatDate} />
        ) : (
          !isGenerating && (
            <div className="no-summaries">
              <AlertCircle size={48} />
              <h4>No Summaries Available</h4>
              <p>Click "Generate Summary" to create a research summary from the knowledge base.</p>
            </div>
          )
        )}
      </div>
    </div>
  );
}

export default RegionDetail;

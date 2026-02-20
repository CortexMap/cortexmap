import { useState, useEffect } from 'react';
import axios from 'axios';
import Cookies from 'js-cookie';
import { ArrowLeft, RefreshCw, Loader2, Clock, CheckCircle, AlertCircle } from 'lucide-react';
import { API_BASE_URL, logger } from '../config';
import SummaryDisplay from './SummaryDisplay';
import './RegionDetail.css';

const STATUS_CONFIG = {
  NotStarted: { label: 'Not Started', color: '#64748b', icon: AlertCircle },
  FetchQueued: { label: 'Fetch Queued', color: '#3b82f6', icon: Clock },
  Fetching: { label: 'Fetching Papers', color: '#3b82f6', icon: Loader2 },
  FetchFailed: { label: 'Fetch Failed', color: '#ef4444', icon: AlertCircle },
  LlmQueued: { label: 'LLM Queued', color: '#8b5cf6', icon: Clock },
  Processing: { label: 'Generating Summary', color: '#8b5cf6', icon: Loader2 },
  Done: { label: 'Complete', color: '#10b981', icon: CheckCircle },
  Invalidated: { label: 'Invalidated', color: '#f59e0b', icon: RefreshCw },
};

// Cookie key for storing batch IDs per region
const BATCH_COOKIE_KEY = 'brainatlas_batch_ids';

// Helper functions for cookie management
const getBatchIdFromCookie = (regionId) => {
  try {
    const cookieData = Cookies.get(BATCH_COOKIE_KEY);
    if (!cookieData) return null;
    
    const batchMap = JSON.parse(cookieData);
    return batchMap[regionId] || null;
  } catch (err) {
    logger.error('Error reading batch ID from cookie:', err);
    return null;
  }
};

const saveBatchIdToCookie = (regionId, batchId) => {
  try {
    const cookieData = Cookies.get(BATCH_COOKIE_KEY);
    const batchMap = cookieData ? JSON.parse(cookieData) : {};
    
    batchMap[regionId] = batchId;
    
    // Store for 24 hours
    Cookies.set(BATCH_COOKIE_KEY, JSON.stringify(batchMap), { expires: 1 });
    logger.info(`Saved batch ID ${batchId} for region ${regionId} to cookie`);
  } catch (err) {
    logger.error('Error saving batch ID to cookie:', err);
  }
};

const removeBatchIdFromCookie = (regionId) => {
  try {
    const cookieData = Cookies.get(BATCH_COOKIE_KEY);
    if (!cookieData) return;
    
    const batchMap = JSON.parse(cookieData);
    delete batchMap[regionId];
    
    if (Object.keys(batchMap).length > 0) {
      Cookies.set(BATCH_COOKIE_KEY, JSON.stringify(batchMap), { expires: 1 });
    } else {
      Cookies.remove(BATCH_COOKIE_KEY);
    }
    logger.info(`Removed batch ID for region ${regionId} from cookie`);
  } catch (err) {
    logger.error('Error removing batch ID from cookie:', err);
  }
};

function RegionDetail({ region, onBack }) {
  const [status, setStatus] = useState(null);
  const [summaries, setSummaries] = useState([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(null);
  const [isGenerating, setIsGenerating] = useState(false);
  const [batchId, setBatchId] = useState(null);
  const [batchStatus, setBatchStatus] = useState(null);

  useEffect(() => {
    logger.info('RegionDetail mounted for region:', region.name, region.id);
    
    // Check if there's a batch ID for this region in cookies
    const savedBatchId = getBatchIdFromCookie(region.id);
    if (savedBatchId) {
      logger.info(`Found saved batch ID ${savedBatchId} for region ${region.id}`);
      setBatchId(savedBatchId);
      setBatchStatus({ status: 'Queued', message: 'Restored batch from cookie' });
    }
    
    fetchRegionStatus();
    fetchRegionSummaries();
  }, [region.id]);

  useEffect(() => {
    // Auto-refresh status if processing
    if (status && ['FetchQueued', 'Fetching', 'LlmQueued', 'Processing'].includes(status.status)) {
      logger.debug(`Auto-refresh active for region ${region.name} (status: ${status.status})`);
      const interval = setInterval(() => {
        fetchRegionStatus();
        fetchRegionSummaries();
      }, 3000);
      return () => {
        logger.debug('Clearing auto-refresh interval');
        clearInterval(interval);
      };
    }
  }, [status]);

  useEffect(() => {
    // Poll batch status if we have a batchId
    if (batchId) {
      logger.debug(`Starting batch status polling for batch ${batchId}`);
      const interval = setInterval(() => {
        fetchBatchStatus();
      }, 2000); // Poll every 2 seconds
      
      return () => {
        logger.debug('Clearing batch status polling interval');
        clearInterval(interval);
      };
    }
  }, [batchId]);

  const fetchRegionStatus = async () => {
    const url = `${API_BASE_URL}/regions/${region.id}/status`;
    logger.api('GET', url);
    
    try {
      const response = await axios.get(url);
      logger.apiSuccess('GET', url, response.data);
      setStatus(response.data);
      setError(null);
    } catch (err) {
      logger.apiError('GET', url, err);
      setError('Failed to fetch region status');
    }
  };

  const fetchRegionSummaries = async () => {
    const url = `${API_BASE_URL}/regions/${region.id}/summaries`;
    logger.api('GET', url);
    
    try {
      const response = await axios.get(url);
      logger.apiSuccess('GET', url, {
        fullResponse: response.data,
        summaryCount: response.data?.summaries?.length || 0
      });
      
      // API returns { summaries: [...] } so we need to access .summaries
      const summariesData = response.data?.summaries || [];
      setSummaries(summariesData);
      logger.info(`Loaded ${summariesData.length} summaries for region ${region.name}`);
      setError(null);
    } catch (err) {
      logger.apiError('GET', url, err);
      // Don't show error for summaries fetch - region might not have summaries yet
      logger.debug('No summaries available yet for this region');
      setSummaries([]);
    }
  };

  const fetchBatchStatus = async () => {
    if (!batchId) return;
    
    const url = `${API_BASE_URL}/batches/${batchId}/status`;
    logger.api('GET', url);
    
    try {
      const response = await axios.get(url);
      logger.apiSuccess('GET', url, response.data);
      setBatchStatus(response.data);
      
      // If batch is complete, stop polling and refresh summaries
      if (response.data.status === 'Completed' || response.data.status === 'Failed') {
        logger.info(`Batch ${batchId} finished with status: ${response.data.status}`);
        
        // Remove from cookie
        removeBatchIdFromCookie(region.id);
        
        setBatchId(null); // Stop polling
        setBatchStatus(null);
        await fetchRegionStatus();
        await fetchRegionSummaries();
      }
    } catch (err) {
      logger.apiError('GET', url, err);
      // Don't show error - batch might not be ready yet
    }
  };

  const handleGenerateSummary = async () => {
    const url = `${API_BASE_URL}/regions/${region.id}/generate`;
    logger.api('POST', url);
    
    try {
      setIsGenerating(true);
      setError(null);
      
      const response = await axios.post(url);
      logger.apiSuccess('POST', url, response.data);
      
      // Store batch_id from response
      if (response.data.batch_id) {
        logger.info('Received batch_id:', response.data.batch_id);
        setBatchId(response.data.batch_id);
        setBatchStatus({ status: 'Queued', message: 'Summary generation started' });
        
        // Save to cookie
        saveBatchIdToCookie(region.id, response.data.batch_id);
      }
      
      // Immediately fetch updated status
      await fetchRegionStatus();
      
      setIsGenerating(false);
      logger.info('Summary generation triggered successfully');
    } catch (err) {
      logger.apiError('POST', url, err);
      setError('Failed to start summary generation');
      setIsGenerating(false);
    }
  };

  const handleInitialSearch = async () => {
    const url = `${API_BASE_URL}/regions/${region.id}/generate`;
    logger.api('POST', url, { action: 'initial generation' });
    
    try {
      setLoading(true);
      setError(null);
      
      const response = await axios.post(url);
      logger.apiSuccess('POST', url, response.data);
      
      // Store batch_id from response
      if (response.data.batch_id) {
        logger.info('Received batch_id:', response.data.batch_id);
        setBatchId(response.data.batch_id);
        setBatchStatus({ status: 'Queued', message: 'Summary generation started' });
        
        // Save to cookie
        saveBatchIdToCookie(region.id, response.data.batch_id);
      }
      
      // Fetch status to get details
      await fetchRegionStatus();
      
      setLoading(false);
      logger.info('Initial summary generation triggered');
    } catch (err) {
      logger.apiError('POST', url, err);
      setError('Failed to initiate summary generation');
      setLoading(false);
    }
  };

  const rgbToHex = (color) => {
    if (!color) return '#6366f1';
    const { red, green, blue } = color;
    return `#${((1 << 24) + (red << 16) + (green << 8) + blue).toString(16).slice(1)}`;
  };

  const formatDate = (dateString) => {
    if (!dateString) return 'N/A';
    return new Date(dateString).toLocaleString();
  };

  const statusConfig = status ? STATUS_CONFIG[status.status] || STATUS_CONFIG.NotStarted : STATUS_CONFIG.NotStarted;
  const StatusIcon = statusConfig.icon;

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
          <h3>Processing Status</h3>
          {status ? (
            <div className="status-content">
              <div className="status-badge" style={{ backgroundColor: statusConfig.color }}>
                <StatusIcon size={20} className={statusConfig.icon === Loader2 ? 'spinning' : ''} />
                <span>{statusConfig.label}</span>
              </div>
              
              <div className="info-rows">
                <div className="info-row">
                  <span className="info-label">Summary Count:</span>
                  <span className="info-value">{status.summary_count || 0}</span>
                </div>
                <div className="info-row">
                  <span className="info-label">Last Fetch:</span>
                  <span className="info-value">{formatDate(status.last_fetch_at)}</span>
                </div>
                <div className="info-row">
                  <span className="info-label">Last Summary:</span>
                  <span className="info-value">{formatDate(status.last_summary_at)}</span>
                </div>
              </div>

              {/* Batch Progress Display - Integrated into Status Card */}
              {batchStatus && batchId && (
                <div className="batch-progress-section">
                  <div className="batch-progress-divider"></div>
                  <div className="batch-progress-header">
                    <Loader2 size={18} className="spinning" />
                    <h4>Summary Generation in Progress</h4>
                  </div>
                  <div className="batch-info">
                    <div className="batch-info-row">
                      <span className="batch-label">Batch ID:</span>
                      <span className="batch-value batch-id">{batchId}</span>
                    </div>
                    <div className="batch-info-row">
                      <span className="batch-label">Status:</span>
                      <span className="batch-value">{batchStatus.status || 'Processing'}</span>
                    </div>
                    {batchStatus.message && (
                      <div className="batch-info-row">
                        <span className="batch-label">Message:</span>
                        <span className="batch-value">{batchStatus.message}</span>
                      </div>
                    )}
                    {batchStatus.progress !== undefined && (
                      <div className="batch-progress-bar-container">
                        <div className="batch-progress-bar">
                          <div 
                            className="batch-progress-fill" 
                            style={{ width: `${batchStatus.progress}%` }}
                          />
                        </div>
                        <span className="batch-progress-text">{batchStatus.progress}%</span>
                      </div>
                    )}
                  </div>
                </div>
              )}
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
            {!status || status.status === 'NotStarted' ? (
              <button
                className="action-button primary"
                onClick={handleInitialSearch}
                disabled={loading}
              >
                {loading ? (
                  <>
                    <Loader2 size={18} className="spinning" />
                    Initiating...
                  </>
                ) : (
                  'Start Summary Generation'
                )}
              </button>
            ) : (
              <button
                className="action-button secondary"
                onClick={handleGenerateSummary}
                disabled={isGenerating || batchId || ['FetchQueued', 'Fetching', 'LlmQueued', 'Processing'].includes(status.status)}
              >
                {isGenerating ? (
                  <>
                    <Loader2 size={18} className="spinning" />
                    Starting...
                  </>
                ) : (
                  <>
                    <RefreshCw size={18} />
                    Generate New Summary
                  </>
                )}
              </button>
            )}
          </div>
        </div>

        {summaries.length > 0 ? (
          <SummaryDisplay summaries={summaries} formatDate={formatDate} />
        ) : (
          <div className="no-summaries">
            {status && ['FetchQueued', 'Fetching', 'LlmQueued', 'Processing'].includes(status.status) ? (
              <>
                <Loader2 size={48} className="spinning" />
                <h4>Generating Summary</h4>
                <p>The system is processing papers and generating a summary for this region.</p>
                <p className="status-detail">Current status: {statusConfig.label}</p>
              </>
            ) : (
              <>
                <AlertCircle size={48} />
                <h4>No Summaries Available</h4>
                <p>This region hasn't been processed yet. Click the button above to start summary generation.</p>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default RegionDetail;

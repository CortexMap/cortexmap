import { useState } from 'react';
import { usePolling } from '../hooks/usePolling';
import { useWorkers } from '../hooks/useWorkers';
import { api } from '../api';
import { PipelineStats } from '../types';
import './StatusPage.css';

export default function StatusPage() {
  const [workerCount, setWorkerCount] = useState(1);
  
  const { data: stats, loading: statsLoading, error: statsError } = usePolling<PipelineStats>(
    () => api.getPipelineStats(),
    2000, // Poll every 2 seconds
    true
  );

  const {
    workers,
    loading: workersLoading,
    error: workersError,
    allocate,
    stop,
    stopAll,
    allocating,
    stopping
  } = useWorkers();

  const formatUptime = (seconds: number) => {
    if (seconds < 60) return `${Math.floor(seconds)}s`;
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
    return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
  };

  const handleAllocate = async () => {
    if (workerCount < 1 || workerCount > 10) {
      alert('Please enter a valid number of workers (1-10)');
      return;
    }
    try {
      await allocate(workerCount);
    } catch (err) {
      // Error already handled in hook
    }
  };

  const handleStop = async (workerId: string) => {
    try {
      await stop(workerId);
    } catch (err) {
      // Error already handled in hook
    }
  };

  const handleStopAll = async () => {
    if (!confirm(`Stop all ${workers.length} workers?`)) return;
    try {
      await stopAll();
    } catch (err) {
      // Error already handled in hook
    }
  };

  const statusItems = stats ? [
    { label: 'Not Started', value: stats.not_started, color: '#9e9e9e', stage: 1 },
    { label: 'Fetch Queued', value: stats.fetch_queued, color: '#2196f3', stage: 2 },
    { label: 'Fetching', value: stats.fetching, color: '#03a9f4', stage: 3 },
    { label: 'Fetch Failed', value: stats.fetch_failed, color: '#f44336', stage: 4 },
    { label: 'LLM Queued', value: stats.llm_queued, color: '#ff9800', stage: 5 },
    { label: 'Processing', value: stats.processing, color: '#ff5722', stage: 6 },
    { label: 'Done', value: stats.done, color: '#4caf50', stage: 7 },
    { label: 'Invalidated', value: stats.invalidated, color: '#9c27b0', stage: 8 }
  ] : [];

  const activeCount = stats ? stats.fetching + stats.processing : 0;

  return (
    <div className="status-page">
      <div className="status-header">
        <h2>Pipeline Status</h2>
        {stats && (
          <div className="status-summary">
            <span className="total-badge">
              {stats.total_regions} total regions
            </span>
            {activeCount > 0 && (
              <span className="active-badge">
                {activeCount} active
              </span>
            )}
          </div>
        )}
      </div>

      {statsLoading && <div className="section-loading">Loading pipeline stats...</div>}
      {statsError && <div className="section-error">{statsError}</div>}
      
      {stats && (
        <>
          {/* Pipeline Timeline */}
          <div className="pipeline-timeline">
            {statusItems.map(item => (
              <div
                key={item.label}
                className={`timeline-stage ${item.value > 0 ? 'active' : ''}`}
                style={{ '--stage-color': item.color } as React.CSSProperties}
              >
                <div className="stage-value">{item.value}</div>
                <div className="stage-label">{item.label}</div>
              </div>
            ))}
          </div>

          {/* Stats Grid */}
          <div className="stats-grid">
            {statusItems.map(item => (
              <div key={item.label} className="stat-card" style={{ borderLeftColor: item.color }}>
                <div className="stat-label">{item.label}</div>
                <div className="stat-value">{item.value}</div>
              </div>
            ))}
          </div>
        </>
      )}

      {/* Workers Section */}
      <div className="workers-section">
        <div className="workers-header">
          <h3>Workers ({workers.length} active)</h3>
          <div className="worker-actions">
            {workers.length > 0 && (
              <button
                className="stop-all-btn"
                onClick={handleStopAll}
                disabled={stopping.size > 0}
              >
                Stop All
              </button>
            )}
            <div className="allocate-group">
              <input
                type="number"
                min="1"
                max="10"
                value={workerCount}
                onChange={e => setWorkerCount(parseInt(e.target.value) || 1)}
                className="worker-count-input"
                disabled={allocating}
              />
              <button
                className="allocate-btn"
                onClick={handleAllocate}
                disabled={allocating}
              >
                {allocating ? 'Adding...' : 'Add Workers'}
              </button>
            </div>
          </div>
        </div>

        {workersError && <div className="section-error">{workersError}</div>}
        {workersLoading && <div className="section-loading">Loading workers...</div>}
        {!workersLoading && workers.length === 0 && (
          <div className="workers-empty">
            No active workers. Set worker count and click "Add Workers" to allocate.
          </div>
        )}

        <div className="workers-list">
          {workers.map(worker => {
            const isStopping = stopping.has(worker.worker_id);
            return (
              <div
                key={worker.worker_id}
                className={`worker-card${isStopping ? ' stopping' : ''}`}
              >
                <div className="worker-header">
                  <span className={`worker-status-badge ${worker.status.toLowerCase()}`}>
                    {worker.status}
                  </span>
                  <button
                    className="worker-stop-btn"
                    onClick={() => handleStop(worker.worker_id)}
                    disabled={isStopping}
                    title="Stop worker"
                  >
                    {isStopping ? 'Stopping...' : 'Delete Worker'}
                  </button>
                </div>
                
                <div className="worker-id" title={worker.worker_id}>
                  {worker.worker_id.slice(0, 8)}...
                </div>
                
                <div className="worker-metrics">
                  <div className="metric">
                    <span className="metric-label">Task:</span>
                    <span className="metric-value">{worker.current_task || 'idle'}</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">Processed:</span>
                    <span className="metric-value">{worker.tasks_processed}</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">Failed:</span>
                    <span className="metric-value">{worker.tasks_failed}</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">Success:</span>
                    <span className="metric-value">{(worker.success_rate * 100).toFixed(0)}%</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">Uptime:</span>
                    <span className="metric-value">{formatUptime(worker.uptime_seconds)}</span>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

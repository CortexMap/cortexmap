import { useState, useEffect } from 'react';
import axios from 'axios';
import { Users, Plus, StopCircle, Loader2, Activity } from 'lucide-react';
import { API_BASE_URL, logger } from '../config';
import './WorkerManagement.css';

function WorkerManagement() {
  const [workers, setWorkers] = useState([]);
  const [loading, setLoading] = useState(true);
  const [allocating, setAllocating] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [workerCount, setWorkerCount] = useState(2);
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    logger.info('WorkerManagement component mounted');
    fetchWorkerStatus();
    const interval = setInterval(() => {
      logger.debug('Auto-refreshing worker status');
      fetchWorkerStatus();
    }, 5000); // Refresh every 5 seconds
    return () => {
      logger.debug('Clearing worker status interval');
      clearInterval(interval);
    };
  }, []);

  const fetchWorkerStatus = async () => {
    const url = `${API_BASE_URL}/workers/status`;
    logger.api('GET', url);
    
    try {
      const response = await axios.get(url);
      logger.apiSuccess('GET', url, response.data);
      setWorkers(response.data || []);
      setLoading(false);
    } catch (err) {
      logger.apiError('GET', url, err);
      logger.warn('Worker status unavailable');
      setLoading(false);
    }
  };

  const handleAllocateWorkers = async () => {
    const url = `${API_BASE_URL}/workers/allocate`;
    const payload = {
      worker_count: workerCount,
      task_timeout_secs: 300,
      max_retry_attempts: 3
    };
    logger.api('POST', url, payload);
    
    try {
      setAllocating(true);
      const response = await axios.post(url, payload);
      logger.apiSuccess('POST', url, response.data);
      logger.info(`Successfully allocated ${workerCount} workers`);
      
      // Refresh worker status after allocation
      await fetchWorkerStatus();
      setAllocating(false);
    } catch (err) {
      logger.apiError('POST', url, err);
      setAllocating(false);
      alert('Failed to allocate workers. Check console for details.');
    }
  };

  const handleStopWorkers = async (workerIds = []) => {
    const url = `${API_BASE_URL}/workers/stop`;
    const payload = { worker_ids: workerIds };
    logger.api('POST', url, payload);
    
    try {
      setStopping(true);
      const response = await axios.post(url, payload);
      logger.apiSuccess('POST', url, response.data);
      logger.info(workerIds.length === 0 ? 'Stopped all workers' : `Stopped ${workerIds.length} workers`);
      
      // Refresh worker status after stopping
      await fetchWorkerStatus();
      setStopping(false);
    } catch (err) {
      logger.apiError('POST', url, err);
      setStopping(false);
      alert('Failed to stop workers. Check console for details.');
    }
  };

  const activeWorkers = workers.filter(w => w.status === 'active' || w.status === 'busy');
  const idleWorkers = workers.filter(w => w.status === 'idle');

  if (loading) {
    return (
      <div className="worker-management loading">
        <Activity size={24} className="spinning" />
        <span>Loading worker information...</span>
      </div>
    );
  }

  return (
    <div className={`worker-management-container ${collapsed ? 'collapsed' : ''}`}>
      <div className="worker-header" onClick={() => setCollapsed(!collapsed)}>
        <div className="worker-title">
          <Users size={24} />
          <h2>Worker Management</h2>
        </div>
        <div className="worker-summary">
          <div className="worker-count">
            <span className="count-value">{workers.length}</span>
            <span className="count-label">Total Workers</span>
          </div>
          <div className="worker-count active">
            <span className="count-value">{activeWorkers.length}</span>
            <span className="count-label">Active</span>
          </div>
          <button className="collapse-btn">{collapsed ? '▼' : '▲'}</button>
        </div>
      </div>

      {!collapsed && (
        <div className="worker-content">
          <div className="worker-controls">
            <div className="allocate-section">
              <label htmlFor="worker-count">Allocate Workers:</label>
              <input
                id="worker-count"
                type="number"
                min="1"
                max="20"
                value={workerCount}
                onChange={(e) => setWorkerCount(parseInt(e.target.value) || 1)}
                className="worker-input"
              />
              <button
                onClick={handleAllocateWorkers}
                disabled={allocating}
                className="action-btn allocate-btn"
              >
                {allocating ? (
                  <>
                    <Loader2 size={18} className="spinning" />
                    Allocating...
                  </>
                ) : (
                  <>
                    <Plus size={18} />
                    Allocate {workerCount} Worker{workerCount > 1 ? 's' : ''}
                  </>
                )}
              </button>
            </div>
            
            {workers.length > 0 && (
              <button
                onClick={() => handleStopWorkers([])}
                disabled={stopping}
                className="action-btn stop-btn"
              >
                {stopping ? (
                  <>
                    <Loader2 size={18} className="spinning" />
                    Stopping...
                  </>
                ) : (
                  <>
                    <StopCircle size={18} />
                    Stop All Workers
                  </>
                )}
              </button>
            )}
          </div>

          {workers.length === 0 ? (
            <div className="no-workers">
              <Users size={48} />
              <h3>No Workers Allocated</h3>
              <p>Allocate workers to start processing brain region summaries.</p>
            </div>
          ) : (
            <div className="workers-grid">
              {workers.map((worker, index) => (
                <div key={worker.id || index} className={`worker-card ${worker.status}`}>
                  <div className="worker-card-header">
                    <span className="worker-id">{worker.id || `Worker ${index + 1}`}</span>
                    <span className={`worker-status-badge ${worker.status}`}>
                      {worker.status || 'unknown'}
                    </span>
                  </div>
                  
                  <div className="worker-details">
                    {worker.current_task && (
                      <div className="worker-detail">
                        <span className="detail-label">Current Task:</span>
                        <span className="detail-value">{worker.current_task}</span>
                      </div>
                    )}
                    {worker.tasks_completed !== undefined && (
                      <div className="worker-detail">
                        <span className="detail-label">Tasks Completed:</span>
                        <span className="detail-value">{worker.tasks_completed}</span>
                      </div>
                    )}
                    {worker.uptime && (
                      <div className="worker-detail">
                        <span className="detail-label">Uptime:</span>
                        <span className="detail-value">{worker.uptime}</span>
                      </div>
                    )}
                  </div>

                  <button
                    onClick={() => handleStopWorkers([worker.id])}
                    disabled={stopping}
                    className="stop-worker-btn"
                  >
                    <StopCircle size={14} />
                    Stop
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default WorkerManagement;

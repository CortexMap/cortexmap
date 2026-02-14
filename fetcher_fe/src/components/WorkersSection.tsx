import React, { useState, useEffect } from 'react';
import { WorkerInfo, QueueStats } from '../types';
import { api } from '../api/backendApi';
import './WorkersSection.css';

const WorkersSection: React.FC = () => {
  const [workers, setWorkers] = useState<WorkerInfo[]>([]);
  const [queueStats, setQueueStats] = useState<QueueStats | null>(null);
  const [error, setError] = useState<string | null>(null);

  const fetchStatus = async () => {
    try {
      const [workerData, statsData] = await Promise.all([
        api.getWorkerStatus(),
        api.getQueueStatus()
      ]);
      setWorkers(workerData);
      setQueueStats(statsData);
      setError(null);
    } catch (err) {
      setError((err as Error).message);
    }
  };

  useEffect(() => {
    fetchStatus();
    const interval = setInterval(fetchStatus, 2000);
    return () => clearInterval(interval);
  }, []);

  const handleAddWorker = async () => {
    try {
      await api.allocateWorkers(1);
      await fetchStatus();
    } catch (err) {
      setError((err as Error).message);
    }
  };

  const handleStopWorker = async (workerId: string) => {
    try {
      await api.stopWorkers([workerId]);
      await fetchStatus();
    } catch (err) {
      setError((err as Error).message);
    }
  };

  const formatTime = (timestamp: number) => {
    const date = new Date(timestamp * 1000);
    return date.toLocaleTimeString();
  };

  return (
    <div className="workers-section">
      {error && <div className="error-message">{error}</div>}
      
      <div className="section-grid">
        <div className="queue-stats-card">
          <h3>Queue Status</h3>
          {queueStats ? (
            <div className="stats-grid">
              <div className="stat-item">
                <span className="stat-value">{queueStats.totalTasks}</span>
                <span className="stat-label">Total</span>
              </div>
              <div className="stat-item pending">
                <span className="stat-value">{queueStats.pendingTasks}</span>
                <span className="stat-label">Pending</span>
              </div>
              <div className="stat-item progress">
                <span className="stat-value">{queueStats.inProgressTasks}</span>
                <span className="stat-label">In Progress</span>
              </div>
              <div className="stat-item completed">
                <span className="stat-value">{queueStats.completedTasks}</span>
                <span className="stat-label">Completed</span>
              </div>
              <div className="stat-item failed">
                <span className="stat-value">{queueStats.failedTasks}</span>
                <span className="stat-label">Failed</span>
              </div>
            </div>
          ) : (
            <div className="loading">Loading...</div>
          )}
        </div>

        <div className="workers-card">
          <div className="workers-header">
            <h3>Workers ({workers.length})</h3>
            <button onClick={handleAddWorker} className="add-worker-btn">+ Add Worker</button>
          </div>
          <div className="workers-list">
            {workers.map((worker) => (
              <div key={worker.workerId} className={`worker-item ${worker.status}`}>
                <div className="worker-info">
                  <div className="worker-id">{worker.workerId.slice(0, 8)}</div>
                  <div className="worker-details">
                    <span className={`status-badge ${worker.status}`}>{worker.status}</span>
                    <span className="tasks-count">{worker.tasksProcessed} tasks</span>
                    <span className="start-time">Started: {formatTime(worker.startedAt)}</span>
                    {worker.currentTask && (
                      <span className="current-task">Processing: {worker.currentTask}</span>
                    )}
                  </div>
                </div>
                <button 
                  onClick={() => handleStopWorker(worker.workerId)} 
                  className="stop-btn"
                  title="Stop worker"
                >
                  ×
                </button>
              </div>
            ))}
            {workers.length === 0 && (
              <div className="empty-workers">No workers running. Click "Add Worker" to start.</div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default WorkersSection;

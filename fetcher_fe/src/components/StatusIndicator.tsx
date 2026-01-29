import React from 'react';
import { TaskStatus } from '../types';
import './StatusIndicator.css';

interface StatusIndicatorProps {
  status: TaskStatus;
  attemptCount: number;
  maxAttempts: number;
}

const StatusIndicator: React.FC<StatusIndicatorProps> = ({ status, attemptCount, maxAttempts }) => {
  const getStatusIcon = () => {
    switch (status) {
      case 'pending':
        return <div className="status-icon pending">⏳</div>;
      case 'in_progress':
        return <div className="status-icon fetching">
          <div className="spinner"></div>
          {attemptCount > 0 && <span className="retry-badge">{attemptCount}</span>}
        </div>;
      case 'completed':
        return <div className="status-icon success">✓</div>;
      case 'failed':
        return <div className="status-icon failed">✗</div>;
      default:
        return null;
    }
  };

  const getStatusText = () => {
    switch (status) {
      case 'pending':
        return 'Pending';
      case 'in_progress':
        return attemptCount > 0 
          ? `Retrying (${attemptCount}/${maxAttempts})` 
          : 'Fetching...';
      case 'completed':
        return 'Success';
      case 'failed':
        return 'Failed';
      default:
        return '';
    }
  };

  return (
    <div className={`status-indicator status-${status}`}>
      {getStatusIcon()}
      <span className="status-text">{getStatusText()}</span>
    </div>
  );
};

export default StatusIndicator;

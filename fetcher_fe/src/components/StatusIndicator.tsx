import React from 'react';
import { FetchStatus } from '../types';
import './StatusIndicator.css';

interface StatusIndicatorProps {
  status: FetchStatus;
  retryCount: number;
}

const StatusIndicator: React.FC<StatusIndicatorProps> = ({ status, retryCount }) => {
  const getStatusIcon = () => {
    switch (status) {
      case FetchStatus.PENDING:
        return <div className="status-icon pending">⏳</div>;
      case FetchStatus.FETCHING:
        return <div className="status-icon fetching">
          <div className="spinner"></div>
        </div>;
      case FetchStatus.RETRYING:
        return <div className="status-icon retrying">
          <div className="spinner"></div>
          <span className="retry-badge">{retryCount}</span>
        </div>;
      case FetchStatus.SUCCESS:
        return <div className="status-icon success">✓</div>;
      case FetchStatus.FAILED:
        return <div className="status-icon failed">✗</div>;
      default:
        return null;
    }
  };

  const getStatusText = () => {
    switch (status) {
      case FetchStatus.PENDING:
        return 'Pending';
      case FetchStatus.FETCHING:
        return 'Fetching...';
      case FetchStatus.RETRYING:
        return `Retrying (${retryCount}/${3})`;
      case FetchStatus.SUCCESS:
        return 'Success';
      case FetchStatus.FAILED:
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

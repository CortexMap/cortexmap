import { useState, useEffect, useRef } from 'react';
import { api } from '../api';
import { BatchStatusResult, RegionPipelineStatus } from '../types';

interface BatchTrackerResult {
  status: RegionPipelineStatus | null;
  message: string;
  completedTasks: number;
  expectedTasks: number;
  isComplete: boolean;
  error: string | null;
  batchData: BatchStatusResult | null;
}

export function useBatchTracker(
  batchId: string | undefined,
  onComplete?: (batchId: string) => void
): BatchTrackerResult {
  const [batchData, setBatchData] = useState<BatchStatusResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const intervalRef = useRef<NodeJS.Timeout | null>(null);
  const onCompleteRef = useRef(onComplete);

  // Keep onComplete callback up to date
  useEffect(() => {
    onCompleteRef.current = onComplete;
  }, [onComplete]);

  useEffect(() => {
    if (!batchId) {
      setBatchData(null);
      setError(null);
      return;
    }

    const pollBatch = async () => {
      try {
        const data = await api.getBatchStatus(batchId);
        setBatchData(data);
        setError(null);

        // Check if batch is complete
        if (data.status === 'Done' || data.status === 'FetchFailed' || data.error) {
          if (intervalRef.current) {
            clearInterval(intervalRef.current);
            intervalRef.current = null;
          }
          
          if (data.status === 'Done' && onCompleteRef.current) {
            onCompleteRef.current(batchId);
          }
        }
      } catch (err) {
        setError((err as Error).message);
      }
    };

    // Poll immediately
    pollBatch();

    // Then poll every 3 seconds with exponential backoff
    let currentInterval = 3000;
    const maxInterval = 60000; // Cap at 60 seconds

    const startPolling = () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
      
      intervalRef.current = setInterval(() => {
        pollBatch();
        
        // Exponential backoff: increase interval gradually
        if (currentInterval < maxInterval) {
          currentInterval = Math.min(currentInterval * 1.2, maxInterval);
          startPolling(); // Restart with new interval
        }
      }, currentInterval);
    };

    startPolling();

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [batchId]);

  return {
    status: batchData?.status || null,
    message: batchData?.message || '',
    completedTasks: batchData?.completed_tasks || 0,
    expectedTasks: batchData?.expected_tasks || 0,
    isComplete: batchData?.status === 'Done' || batchData?.status === 'FetchFailed' || false,
    error: error || batchData?.error || null,
    batchData
  };
}

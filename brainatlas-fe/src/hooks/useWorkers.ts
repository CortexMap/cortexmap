import { useState, useCallback } from 'react';
import { api } from '../api';
import { WorkerStatus } from '../types';
import { usePolling } from './usePolling';

interface WorkersResult {
  workers: WorkerStatus[];
  loading: boolean;
  error: string | null;
  allocate: (count: number) => Promise<void>;
  stop: (workerId: string) => Promise<void>;
  stopAll: () => Promise<void>;
  refresh: () => void;
  allocating: boolean;
  stopping: Set<string>;
}

export function useWorkers(): WorkersResult {
  const [allocating, setAllocating] = useState(false);
  const [stopping, setStopping] = useState<Set<string>>(new Set());
  const [actionError, setActionError] = useState<string | null>(null);

  const { data: workers, loading, error, refresh } = usePolling<WorkerStatus[]>(
    () => api.getWorkerStatus(),
    2000, // Poll every 2 seconds
    true
  );

  const allocate = useCallback(async (count: number) => {
    setAllocating(true);
    setActionError(null);
    try {
      await api.allocateWorkers(count);
      refresh();
    } catch (err) {
      setActionError((err as Error).message);
      throw err;
    } finally {
      setAllocating(false);
    }
  }, [refresh]);

  const stop = useCallback(async (workerId: string) => {
    setStopping(prev => new Set(prev).add(workerId));
    setActionError(null);
    try {
      await api.stopWorker(workerId);
      // Wait a bit before refreshing to allow optimistic UI update
      setTimeout(refresh, 500);
    } catch (err) {
      setActionError((err as Error).message);
      throw err;
    } finally {
      setStopping(prev => {
        const next = new Set(prev);
        next.delete(workerId);
        return next;
      });
    }
  }, [refresh]);

  const stopAll = useCallback(async () => {
    if (!workers) return;
    
    const allIds = workers.map(w => w.worker_id);
    setStopping(new Set(allIds));
    setActionError(null);
    try {
      await api.stopAllWorkers();
      setTimeout(refresh, 500);
    } catch (err) {
      setActionError((err as Error).message);
      throw err;
    } finally {
      setStopping(new Set());
    }
  }, [workers, refresh]);

  return {
    workers: workers || [],
    loading,
    error: error || actionError,
    allocate,
    stop,
    stopAll,
    refresh,
    allocating,
    stopping
  };
}

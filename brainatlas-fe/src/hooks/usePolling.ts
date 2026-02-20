import { useState, useEffect, useRef, useCallback } from 'react';

interface PollingResult<T> {
  data: T | null;
  loading: boolean;
  error: string | null;
  refresh: () => void;
}

export function usePolling<T>(
  fetcher: () => Promise<T>,
  intervalMs: number,
  enabled: boolean = true
): PollingResult<T> {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isVisible, setIsVisible] = useState(true);
  
  const intervalRef = useRef<NodeJS.Timeout | null>(null);
  const isMountedRef = useRef(true);
  const fetcherRef = useRef(fetcher);

  // Update fetcher ref without triggering re-render
  useEffect(() => {
    fetcherRef.current = fetcher;
  }, [fetcher]);

  const fetchData = useCallback(async () => {
    // Only fetch if page is visible
    if (!document.hidden) {
      try {
        setError(null);
        const result = await fetcherRef.current();
        if (isMountedRef.current) {
          setData(result);
          setLoading(false);
        }
      } catch (err) {
        if (isMountedRef.current) {
          setError((err as Error).message);
          setLoading(false);
        }
      }
    }
  }, []); // No dependencies - uses refs

  const refresh = useCallback(() => {
    setLoading(true);
    fetchData();
  }, [fetchData]);

  // Handle page visibility changes
  useEffect(() => {
    const handleVisibilityChange = () => {
      setIsVisible(!document.hidden);
      // Fetch immediately when page becomes visible again
      if (!document.hidden && enabled) {
        fetchData();
      }
    };

    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => document.removeEventListener('visibilitychange', handleVisibilityChange);
  }, [enabled, fetchData]);

  useEffect(() => {
    isMountedRef.current = true;

    if (enabled && isVisible) {
      // Fetch immediately
      fetchData();

      // Then poll at interval
      intervalRef.current = setInterval(fetchData, intervalMs);
    }

    return () => {
      isMountedRef.current = false;
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [enabled, intervalMs, isVisible, fetchData]);

  return { data, loading, error, refresh };
}

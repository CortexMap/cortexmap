import { useState, useCallback, useRef, useEffect } from 'react';
import { PaperState } from '../types';
import { api } from '../api/backendApi';

const POLL_INTERVAL = 200; // Poll every 200ms as requested

export const usePaperFetcher = () => {
  const [papers, setPapers] = useState<Map<string, PaperState>>(new Map());
  const [isSearching, setIsSearching] = useState(false);
  const [pmcIds, setPmcIds] = useState<string[]>([]);
  const [lastEnqueueResponse, setLastEnqueueResponse] = useState<any>(null);
  const pollIntervalRef = useRef<NodeJS.Timeout | null>(null);

  // Poll for updates on all tracked PMC IDs
  const pollForUpdates = useCallback(async () => {
    if (pmcIds.length === 0) return;

    try {
      // Get queue status with recent tasks
      const queueStats = await api.getQueueStatus();
      
      // Update papers from recent_tasks if available
      if (queueStats.recentTasks && queueStats.recentTasks.length > 0) {
        queueStats.recentTasks.forEach((task) => {
          if (pmcIds.includes(task.pmcId)) {
            setPapers(prev => {
              const newPapers = new Map(prev);
              
              newPapers.set(task.pmcId, {
                pmcId: task.pmcId,
                status: task.status as any,
                components: new Map(),
                lastUpdated: Date.now(),
                summary: task.summaryContent,
                abstract: task.abstractContent
              });
              
              return newPapers;
            });
          }
        });
      }
    } catch (error) {
      console.error('Failed to poll for updates:', error);
    }
  }, [pmcIds]);

  // Start polling when we have PMC IDs
  useEffect(() => {
    if (pmcIds.length > 0 && !pollIntervalRef.current) {
      // Initial fetch
      pollForUpdates();

      // Set up polling
      pollIntervalRef.current = setInterval(pollForUpdates, POLL_INTERVAL);
    }

    return () => {
      if (pollIntervalRef.current) {
        clearInterval(pollIntervalRef.current);
        pollIntervalRef.current = null;
      }
    };
  }, [pmcIds, pollForUpdates]);

  // Main search function
  const search = useCallback(async (query: string, pageSize: number = 10) => {
    setIsSearching(true);
    setPapers(new Map());
    setPmcIds([]);
    setLastEnqueueResponse(null);

    try {
      console.log(`🔍 Enqueueing query: "${query}"`);
      
      // Enqueue the query
      const response = await api.enqueueQuery(query, pageSize, 0);
      
      if (!response.success) {
        throw new Error(response.errorMessage || 'Failed to enqueue query');
      }

      console.log(`✅ Enqueued ${response.tasksEnqueued} tasks`);
      console.log(`📋 PMC IDs:`, response.pmcIds);

      // Store response for QueryPage
      setLastEnqueueResponse(response);

      // Store PMC IDs to start polling
      setPmcIds(response.pmcIds || []);

      // Initialize paper states
      const initialPapers = new Map<string, PaperState>();
      (response.pmcIds || []).forEach((pmcId) => {
        initialPapers.set(pmcId, {
          pmcId,
          status: 'pending',
          components: new Map(),
          lastUpdated: Date.now(),
        });
      });
      setPapers(initialPapers);

    } catch (error) {
      console.error('❌ Search failed:', error);
      throw error;
    } finally {
      setIsSearching(false);
    }
  }, []);

  return {
    papers: Array.from(papers.values()),
    isSearching,
    search,
    lastEnqueueResponse
  };
};

import { useState, useCallback, useRef, useEffect } from 'react';
import { PaperState, QueueStats, ComponentType, TaskStatus } from '../types';
import { api } from '../api/backendApi';

const POLL_INTERVAL = 200; // Poll every 200ms as requested

export const usePaperFetcher = () => {
  const [papers, setPapers] = useState<Map<string, PaperState>>(new Map());
  const [isSearching, setIsSearching] = useState(false);
  const [queueStats, setQueueStats] = useState<QueueStats | null>(null);
  const [pmcIds, setPmcIds] = useState<string[]>([]);
  const pollIntervalRef = useRef<NodeJS.Timeout | null>(null);

  // Update paper state from task details
  const updatePaperState = useCallback((pmcId: string, taskDetails: any) => {
    setPapers(prev => {
      const newPapers = new Map(prev);
      
      const componentsMap = new Map<ComponentType, any>();
      if (taskDetails.components) {
        taskDetails.components.forEach((comp: any) => {
          componentsMap.set(comp.componentType as ComponentType, {
            componentType: comp.componentType,
            status: comp.status,
            attemptCount: comp.attemptCount,
            maxAttempts: comp.maxAttempts,
            s3Key: comp.s3Key,
            errorMessage: comp.errorMessage,
          });
        });
      }

      newPapers.set(pmcId, {
        pmcId,
        status: taskDetails.status as TaskStatus,
        components: componentsMap,
        lastUpdated: Date.now(),
      });

      return newPapers;
    });
  }, []);

  // Poll for updates on all tracked PMC IDs
  const pollForUpdates = useCallback(async () => {
    if (pmcIds.length === 0) return;

    try {
      // Fetch task details for all PMC IDs
      const detailsMap = await api.getMultipleTaskDetails(pmcIds);
      
      detailsMap.forEach((details, pmcId) => {
        if (details.found) {
          updatePaperState(pmcId, details);
        }
      });

      // Also update queue stats
      const stats = await api.getQueueStatus();
      setQueueStats(stats);
    } catch (error) {
      console.error('Failed to poll for updates:', error);
    }
  }, [pmcIds, updatePaperState]);

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
    setQueueStats(null);

    try {
      console.log(`🔍 Enqueueing query: "${query}"`);
      
      // Enqueue the query
      const response = await api.enqueueQuery(query, pageSize, 3);
      
      if (!response.success) {
        throw new Error(response.errorMessage || 'Failed to enqueue query');
      }

      console.log(`✅ Enqueued ${response.tasksEnqueued} tasks`);
      console.log(`📋 PMC IDs:`, response.pmcIds);

      // Store PMC IDs to start polling
      setPmcIds(response.pmcIds);

      // Initialize paper states
      const initialPapers = new Map<string, PaperState>();
      response.pmcIds.forEach((pmcId) => {
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

  // Allocate workers
  const allocateWorkers = useCallback(async (count: number) => {
    try {
      console.log(`🔧 Allocating ${count} workers...`);
      const response = await api.allocateWorkers(count);
      
      if (!response.success) {
        throw new Error(response.errorMessage || 'Failed to allocate workers');
      }

      console.log(`✅ Allocated workers:`, response.workerIds);
      return response.workerIds;
    } catch (error) {
      console.error('❌ Failed to allocate workers:', error);
      throw error;
    }
  }, []);

  // Stop all workers
  const stopWorkers = useCallback(async () => {
    try {
      console.log(`🛑 Stopping workers...`);
      const response = await api.stopWorkers();
      console.log(`✅ Stopped ${response.workersStopped} workers`);
      return response.workersStopped;
    } catch (error) {
      console.error('❌ Failed to stop workers:', error);
      throw error;
    }
  }, []);

  return {
    papers: Array.from(papers.values()),
    isSearching,
    search,
    queueStats,
    allocateWorkers,
    stopWorkers,
  };
};

import { useState, useCallback, useRef, useEffect } from 'react';
import { PaperFetchState, FetchStatus, RetryQueueItem } from '../types';
import { fetchMetadata, fetchAbstract, fetchPDF } from '../api/mockApi';

const MAX_RETRIES = 3;
const POLL_INTERVAL = 200;

export const usePaperFetcher = () => {
  const [papers, setPapers] = useState<Map<string, PaperFetchState>>(new Map());
  const [isSearching, setIsSearching] = useState(false);
  const [retryQueue, setRetryQueue] = useState<RetryQueueItem[]>([]);
  const pollIntervalRef = useRef<NodeJS.Timeout | null>(null);
  const retryTimeoutRef = useRef<NodeJS.Timeout | null>(null);

  // Initialize paper state
  const initializePaper = useCallback((paperId: string): PaperFetchState => {
    return {
      paper: { id: paperId },
      components: {
        metadata: { name: 'metadata', status: FetchStatus.PENDING, retryCount: 0 },
        abstract: { name: 'abstract', status: FetchStatus.PENDING, retryCount: 0 },
        pdf: { name: 'pdf', status: FetchStatus.PENDING, retryCount: 0 }
      },
      overallStatus: FetchStatus.PENDING
    };
  }, []);

  // Update component status
  const updateComponentStatus = useCallback((
    paperId: string,
    componentName: 'metadata' | 'abstract' | 'pdf',
    status: FetchStatus,
    data?: any,
    error?: string
  ) => {
    setPapers(prev => {
      const newPapers = new Map(prev);
      const paperState = newPapers.get(paperId);
      
      if (!paperState) return prev;

      const updatedPaper = { ...paperState };
      updatedPaper.components[componentName] = {
        ...updatedPaper.components[componentName],
        status,
        error
      };

      // Update paper data if provided
      if (data) {
        if (componentName === 'metadata') {
          updatedPaper.paper = { ...updatedPaper.paper, metadata: data, pmid: data.pmid };
        } else if (componentName === 'abstract') {
          updatedPaper.paper = { ...updatedPaper.paper, abstract: data };
        } else if (componentName === 'pdf') {
          updatedPaper.paper = { ...updatedPaper.paper, pdfUrl: data };
        }
      }

      // Update overall status
      const statuses = Object.values(updatedPaper.components).map(c => c.status);
      if (statuses.every(s => s === FetchStatus.SUCCESS)) {
        updatedPaper.overallStatus = FetchStatus.SUCCESS;
      } else if (statuses.some(s => s === FetchStatus.FAILED)) {
        updatedPaper.overallStatus = FetchStatus.FAILED;
      } else if (statuses.some(s => s === FetchStatus.FETCHING || s === FetchStatus.RETRYING)) {
        updatedPaper.overallStatus = FetchStatus.FETCHING;
      }

      newPapers.set(paperId, updatedPaper);
      return newPapers;
    });
  }, []);

  // Fetch a single component
  const fetchComponent = useCallback(async (
    paperId: string,
    componentName: 'metadata' | 'abstract' | 'pdf',
    retryCount: number = 0
  ) => {
    const status = retryCount > 0 ? FetchStatus.RETRYING : FetchStatus.FETCHING;
    updateComponentStatus(paperId, componentName, status);

    try {
      let data;
      if (componentName === 'metadata') {
        data = await fetchMetadata(paperId, retryCount);
      } else if (componentName === 'abstract') {
        data = await fetchAbstract(paperId, retryCount);
      } else {
        data = await fetchPDF(paperId, retryCount);
      }

      updateComponentStatus(paperId, componentName, FetchStatus.SUCCESS, data);
      return true;
    } catch (error) {
      console.error(`Failed to fetch ${componentName} for ${paperId}:`, error);
      
      if (retryCount < MAX_RETRIES) {
        // Add to retry queue
        setRetryQueue(prev => [...prev, { paperId, componentName, retryCount: retryCount + 1 }]);
        updateComponentStatus(paperId, componentName, FetchStatus.RETRYING, undefined, (error as Error).message);
      } else {
        updateComponentStatus(paperId, componentName, FetchStatus.FAILED, undefined, (error as Error).message);
      }
      return false;
    }
  }, [updateComponentStatus]);

  // Process retry queue
  const processRetryQueue = useCallback(async () => {
    if (retryQueue.length === 0) return;

    const item = retryQueue[0];
    setRetryQueue(prev => prev.slice(1));

    console.log(`Retrying ${item.componentName} for ${item.paperId} (attempt ${item.retryCount + 1}/${MAX_RETRIES})`);
    
    await fetchComponent(item.paperId, item.componentName, item.retryCount);
  }, [retryQueue, fetchComponent]);

  // Poll and process retry queue
  useEffect(() => {
    if (retryQueue.length > 0 && !retryTimeoutRef.current) {
      retryTimeoutRef.current = setTimeout(() => {
        processRetryQueue();
        retryTimeoutRef.current = null;
      }, POLL_INTERVAL);
    }

    return () => {
      if (retryTimeoutRef.current) {
        clearTimeout(retryTimeoutRef.current);
        retryTimeoutRef.current = null;
      }
    };
  }, [retryQueue, processRetryQueue]);

  // Start fetching papers
  const startFetchingPapers = useCallback(async (paperIds: string[]) => {
    // Initialize all papers
    const initialPapers = new Map<string, PaperFetchState>();
    paperIds.forEach(id => {
      initialPapers.set(id, initializePaper(id));
    });
    setPapers(initialPapers);

    // Start fetching all components for all papers
    for (const paperId of paperIds) {
      fetchComponent(paperId, 'metadata', 0);
      fetchComponent(paperId, 'abstract', 0);
      fetchComponent(paperId, 'pdf', 0);
    }
  }, [initializePaper, fetchComponent]);

  // Main search function
  const search = useCallback(async (query: string) => {
    setIsSearching(true);
    setPapers(new Map());
    setRetryQueue([]);

    try {
      // Import search function dynamically to avoid circular dependency
      const { searchPapers } = await import('../api/mockApi');
      const paperIds = await searchPapers(query);
      
      await startFetchingPapers(paperIds);
    } catch (error) {
      console.error('Search failed:', error);
    } finally {
      setIsSearching(false);
    }
  }, [startFetchingPapers]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (pollIntervalRef.current) {
        clearInterval(pollIntervalRef.current);
      }
      if (retryTimeoutRef.current) {
        clearTimeout(retryTimeoutRef.current);
      }
    };
  }, []);

  return {
    papers: Array.from(papers.values()),
    isSearching,
    search,
    retryQueueLength: retryQueue.length
  };
};

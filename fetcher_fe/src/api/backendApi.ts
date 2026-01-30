import {
  TaskDetails,
  QueueStats,
  WorkerInfo,
  ComponentType,
  ComponentStatus,
} from '../types';

// API base URL - will be proxied through Vite dev server
const API_BASE = '/api';

interface EnqueueResponse {
  success: boolean;
  tasksEnqueued: number;
  pmcIds: string[];
  errorMessage?: string;
}

interface AllocateWorkersResponse {
  success: boolean;
  workerIds: string[];
  errorMessage?: string;
}

class BackendAPI {
  // Enqueue a query to fetch papers
  async enqueueQuery(
    query: string,
    pageSize: number = 10,
    maxRetryAttempts: number = 3
  ): Promise<EnqueueResponse> {
    const response = await fetch(`${API_BASE}/enqueue`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        query,
        pageSize,
        maxRetryAttempts,
      }),
    });

    if (!response.ok) {
      throw new Error(`Failed to enqueue query: ${response.statusText}`);
    }

    return response.json();
  }

  // Get overall queue statistics
  async getQueueStatus(): Promise<QueueStats> {
    const response = await fetch(`${API_BASE}/queue/status`);

    if (!response.ok) {
      throw new Error(`Failed to get queue status: ${response.statusText}`);
    }

    return response.json();
  }

  // Get detailed status for a specific task
  async getTaskDetails(pmcId: string): Promise<TaskDetails> {
    const response = await fetch(`${API_BASE}/task/${pmcId}`);

    if (!response.ok) {
      throw new Error(`Failed to get task details: ${response.statusText}`);
    }

    return response.json();
  }

  // Get details for multiple tasks in parallel
  async getMultipleTaskDetails(pmcIds: string[]): Promise<Map<string, TaskDetails>> {
    const results = new Map<string, TaskDetails>();

    // Batch request for better performance
    const response = await fetch(`${API_BASE}/tasks/batch`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ pmcIds }),
    });

    if (!response.ok) {
      // Fallback to individual requests if batch not supported
      const promises = pmcIds.map(async (pmcId) => {
        try {
          const details = await this.getTaskDetails(pmcId);
          return { pmcId, details };
        } catch (error) {
          console.error(`Failed to get details for ${pmcId}:`, error);
          return null;
        }
      });

      const responses = await Promise.all(promises);
      responses.forEach((response) => {
        if (response) {
          results.set(response.pmcId, response.details);
        }
      });

      return results;
    }

    const data = await response.json();
    Object.entries(data).forEach(([pmcId, details]) => {
      results.set(pmcId, details as TaskDetails);
    });

    return results;
  }

  // Allocate workers
  async allocateWorkers(
    workerCount: number,
    taskTimeoutSecs: number = 120,
    maxRetryAttempts: number = 3
  ): Promise<AllocateWorkersResponse> {
    const response = await fetch(`${API_BASE}/workers/allocate`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        workerCount,
        taskTimeoutSecs,
        maxRetryAttempts,
      }),
    });

    if (!response.ok) {
      throw new Error(`Failed to allocate workers: ${response.statusText}`);
    }

    return response.json();
  }

  // Get worker status
  async getWorkerStatus(): Promise<WorkerInfo[]> {
    const response = await fetch(`${API_BASE}/workers/status`);

    if (!response.ok) {
      throw new Error(`Failed to get worker status: ${response.statusText}`);
    }

    const data = await response.json();
    return data.workers || [];
  }

  // Stop all workers
  async stopWorkers(workerIds: string[] = []): Promise<{ success: boolean; workersStopped: number }> {
    const response = await fetch(`${API_BASE}/workers/stop`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ workerIds }),
    });

    if (!response.ok) {
      throw new Error(`Failed to stop workers: ${response.statusText}`);
    }

    return response.json();
  }
}

// Export singleton instance
export const api = new BackendAPI();

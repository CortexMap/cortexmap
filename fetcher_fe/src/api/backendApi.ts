import {
  TaskDetails,
  QueueStats,
  WorkerInfo,
} from '../types';

// API base URL - proxied through Vite dev server to avoid CORS
const API_BASE = '/fetcher-be/api';

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
    maxRetryAttempts: number = 0
  ): Promise<EnqueueResponse> {
    const response = await fetch(`${API_BASE}/queue/enqueue`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ 
        query, 
        page_size: pageSize, 
        max_retry_attempts: maxRetryAttempts 
      }),
    });

    if (!response.ok) throw new Error(`Failed to enqueue: ${response.statusText}`);
    
    const data = await response.json();
    // Convert snake_case to camelCase
    return {
      success: data.success,
      tasksEnqueued: data.tasks_enqueued,
      pmcIds: data.pmc_ids || [],
      errorMessage: data.error_message
    };
  }

  // Get overall queue statistics
  async getQueueStatus(): Promise<QueueStats> {
    const response = await fetch(`${API_BASE}/queue/status`);
    if (!response.ok) throw new Error(`Failed to get status: ${response.statusText}`);
    
    const data = await response.json();
    return {
      totalTasks: data.total_tasks,
      pendingTasks: data.pending_tasks,
      inProgressTasks: data.in_progress_tasks,
      completedTasks: data.completed_tasks,
      failedTasks: data.failed_tasks,
      activeWorkers: data.active_workers,
      recentTasks: (data.recent_tasks || []).map((t: any) => ({
        pmcId: t.pmc_id,
        status: t.status,
        createdAt: t.created_at,
        updatedAt: t.updated_at,
        workerId: t.worker_id,
        componentsCompleted: t.components_completed,
        totalComponents: t.total_components,
        summaryContent: t.summary_content,
        abstractContent: t.abstract_content
      }))
    };
  }

  // Get detailed status for a specific task
  async getTaskDetails(pmcId: string): Promise<TaskDetails> {
    const response = await fetch(`${API_BASE}/queue/task/${pmcId}`);
    if (!response.ok) {
      console.error(`Failed to get task ${pmcId}: ${response.status}`);
      throw new Error(`Failed to get task: ${response.statusText}`);
    }
    
    const data = await response.json();
    console.log(`Task ${pmcId} response:`, data);
    
    return {
      found: data.found !== false,
      pmcId: data.pmc_id || pmcId,
      status: data.status,
      components: (data.components || []).map((c: any) => ({
        componentType: c.component_type,
        status: c.status,
        attemptCount: c.attempt_count,
        maxAttempts: c.max_attempts,
        s3Key: c.s3_key,
        errorMessage: c.error_message
      })),
      errorMessage: data.error_message,
      summaryContent: data.summary_content,
      abstractContent: data.abstract_content
    };
  }

  // Get details for multiple tasks in parallel
  async getMultipleTaskDetails(pmcIds: string[]): Promise<Map<string, TaskDetails>> {
    const results = new Map<string, TaskDetails>();
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
      if (response) results.set(response.pmcId, response.details);
    });

    return results;
  }

  // Allocate workers
  async allocateWorkers(
    workerCount: number,
    taskTimeoutSecs: number = 120,
    maxRetryAttempts: number = 0
  ): Promise<AllocateWorkersResponse> {
    const response = await fetch(`${API_BASE}/queue/workers/allocate`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ 
        worker_count: workerCount, 
        task_timeout_secs: taskTimeoutSecs, 
        max_retry_attempts: maxRetryAttempts 
      }),
    });

    if (!response.ok) throw new Error(`Failed to allocate: ${response.statusText}`);
    
    const data = await response.json();
    return {
      success: data.success,
      workerIds: data.worker_ids || [],
      errorMessage: data.error_message
    };
  }

  // Get worker status
  async getWorkerStatus(): Promise<WorkerInfo[]> {
    const response = await fetch(`${API_BASE}/queue/workers/status`);
    if (!response.ok) throw new Error(`Failed to get workers: ${response.statusText}`);
    
    const data = await response.json();
    return (data.workers || []).map((w: any) => ({
      workerId: w.worker_id,
      status: w.status,
      currentTask: w.current_task,
      tasksProcessed: w.tasks_processed,
      startedAt: w.started_at
    }));
  }

  // Stop all workers
  async stopWorkers(workerIds: string[] = []): Promise<{ success: boolean; workersStopped: number }> {
    const response = await fetch(`${API_BASE}/queue/workers/stop`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ worker_ids: workerIds }),
    });

    if (!response.ok) throw new Error(`Failed to stop: ${response.statusText}`);
    
    const data = await response.json();
    return {
      success: data.success,
      workersStopped: data.workers_stopped
    };
  }

  // Fetch content from S3 key
  async fetchS3Content(s3Key: string): Promise<string> {
    try {
      // Assuming backend provides an endpoint to fetch S3 content
      const response = await fetch(`${API_BASE}/content/${encodeURIComponent(s3Key)}`);
      if (!response.ok) return '';
      return await response.text();
    } catch (error) {
      console.error(`Failed to fetch S3 content for ${s3Key}:`, error);
      return '';
    }
  }
}

// Export singleton instance
export const api = new BackendAPI();

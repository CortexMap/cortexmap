import { QueueStats } from '../types';

export interface QueryHistory {
  id: string;
  query: string;
  timestamp: number;
  tasksEnqueued: number;
  pmcIds: string[];
  status: 'pending' | 'in_progress' | 'completed' | 'failed';
}

export interface TaskBreakdown {
  tasksWithErrors: number;
  tasksPendingRetry: number;
  tasksInProgressOver5min: number;
  averageCompletionTimeSecs: number;
  oldestPendingTaskAge: string;
}

export interface ComponentStatistics {
  totalSummaryCompleted: number;
  totalAbstractCompleted: number;
  totalPdfCompleted: number;
  totalSummaryFailed: number;
  totalAbstractFailed: number;
  totalPdfFailed: number;
  totalComponentsPending: number;
}

export interface WorkerStatistics {
  totalWorkersActive: number;
  totalWorkersIdle: number;
  averageTasksPerWorker: number;
  mostProductiveWorkerId: string;
  mostProductiveWorkerTaskCount: number;
}

export interface RecentTask {
  pmcId: string;
  status: string;
  createdAt: number;
  updatedAt: number;
  workerId: string;
  componentsCompleted: number;
  totalComponents: number;
}

export interface EnhancedQueueStats extends QueueStats {
  taskBreakdown?: TaskBreakdown;
  componentStats?: ComponentStatistics;
  workerStats?: WorkerStatistics;
  recentTasks?: RecentTask[];
}

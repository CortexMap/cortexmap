// Status types matching the proto definition
export type ComponentType = 'summary' | 'abstract' | 'pdf';
export type TaskStatus = 'pending' | 'in_progress' | 'completed' | 'failed';

// Component status from backend
export interface ComponentStatus {
  componentType: ComponentType;
  status: TaskStatus;
  attemptCount: number;
  maxAttempts: number;
  s3Key?: string;
  errorMessage?: string;
}

// Task details from backend
export interface TaskDetails {
  found: boolean;
  pmcId: string;
  status: TaskStatus;
  components: ComponentStatus[];
  errorMessage?: string;
}

// Queue statistics
export interface QueueStats {
  totalTasks: number;
  pendingTasks: number;
  inProgressTasks: number;
  completedTasks: number;
  failedTasks: number;
  activeWorkers: number;
}

// Worker information
export interface WorkerInfo {
  workerId: string;
  status: 'running' | 'idle' | 'stopped';
  currentTask?: string;
  tasksProcessed: number;
  startedAt: number;
}

// Local UI state for tracking papers
export interface PaperState {
  pmcId: string;
  status: TaskStatus;
  components: Map<ComponentType, ComponentStatus>;
  lastUpdated: number;
}

export interface ConfigEntry {
  key: string;
  value: string;
  description: string;
  updated_at: string;
}

export interface ConfigEntryUpdate {
  key: string;
  value: string;
}

export interface RegionSummary {
  summary: string;
  created_at: string;
  batch_id: string;
  sources: SummarySource[];
}

export interface SummarySource {
  chunk_id: string;
  pmc_id?: string;
  uid?: string;
  source_query?: string;
}

export interface SearchRegionResponse {
  status: string;
  summaries: RegionSummary[];
}

export interface PipelineStats {
  total_regions: number;
  not_started: number;
  fetch_queued: number;
  fetching: number;
  fetch_failed: number;
  llm_queued: number;
  processing: number;
  done: number;
  invalidated: number;
}

export interface ChatMessage {
  role: 'user' | 'assistant';
  content: string;
  timestamp: string;
  batchId?: string; // For tracking in-progress batches
  regionId?: string; // For resume-on-refresh
}

export interface RegionMapping {
  id: string;
  region_id: number;
  name: string;
  acronym: string;
  parent_acronym: string;
}

export interface WorkerStatus {
  worker_id: string;
  status: string; // "running", "idle", "stopped"
  current_task: string | null; // PMC ID or null
  tasks_processed: number;
  started_at: number; // Unix timestamp
  worker_version: string | null;
  last_heartbeat_at: number | null; // Unix timestamp
  uptime_seconds: number;
  tasks_failed: number;
  success_rate: number; // 0.0 to 1.0
}

// Worker management request/response types
export interface AllocateWorkersRequest {
  worker_count: number;
  task_timeout_secs: number;
  max_retry_attempts: number;
}

export interface WorkerAllocationResponse {
  success: boolean;
  worker_ids: string[];
  error_message: string | null;
}

export interface StopWorkersRequest {
  worker_ids: string[]; // Empty array means stop all
}

export interface WorkerStopResponse {
  success: boolean;
  workers_stopped: number;
  error_message: string | null;
}

// Pipeline status types
export type RegionPipelineStatus = 
  | 'NotStarted'
  | 'FetchQueued'
  | 'Fetching'
  | 'FetchFailed'
  | 'LlmQueued'
  | 'Processing'
  | 'Done'
  | 'Invalidated';

export interface BatchStatusResult {
  batch_id: string;
  status: RegionPipelineStatus;
  message: string; // e.g., "Fetching papers: 10/20 complete"
  error: string | null;
  expected_tasks: number;
  completed_tasks: number | null;
  created_at: string;
}

export interface RegionStatusResult {
  region_id: string;
  status: RegionPipelineStatus;
  last_fetch_at: string | null;
  last_summary_at: string | null;
  summary_count: number;
  current_priority: number | null;
}

export interface GenerateSummaryResult {
  batch_id: string;
  query_count: number;
  task_count: number;
}

export interface ChunkSourceResponse {
  chunk_id: string;
  chunk_text: string;
  source_s3_key: string | null;
  source_pmc_id: string | null;
  source_uid: string | null;
  source_query: string | null;
  char_start: number | null;
  char_end: number | null;
}

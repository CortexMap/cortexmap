/** Region from cortexmap orch API /regions endpoint */
export interface CortexmapRegion {
  id: string; // UUID
  region_id: number; // Allen structure_id
  name: string;
  acronym: string | null;
  color: { red: number; green: number; blue: number } | null;
  structure_order: number | null;
  parent_region_id: number | null;
  parent_acronym: string | null;
}

/** Summary from cortexmap orch API */
export interface RegionSummary {
  summary: string;
  created_at: string;
  batch_id: string;
  sources: SummarySource[];
}

export interface SummarySource {
  chunk_id: string;
  pmc_id: string | null;
  uid: string | null;
  source_query: string | null;
}

/** Batch status from orch API */
export interface BatchStatus {
  batch_id: string;
  status: string;
  message?: string;
  error?: string;
  expected_tasks?: number;
  completed_tasks?: number | null;
  created_at: string;
}

/** Region pipeline status */
export interface RegionStatus {
  region_id: string;
  status: 'NotStarted' | 'FetchQueued' | 'Fetching' | 'FetchFailed' | 'LlmQueued' | 'Processing' | 'Done';
  last_fetch_at: string | null;
  last_summary_at: string | null;
  summary_count: number;
  current_priority: number | null;
}

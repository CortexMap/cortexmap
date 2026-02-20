import {
  ConfigEntry,
  ConfigEntryUpdate,
  SearchRegionResponse,
  PipelineStats,
  RegionMapping,
  WorkerStatus,
  WorkerAllocationResponse,
  WorkerStopResponse,
  BatchStatusResult,
  RegionStatusResult,
  GenerateSummaryResult,
  ChunkSourceResponse
} from './types';

const API_BASE = 'https://capstone.ssdd.dev/orch/api';

export const api = {
  async getConfig(): Promise<ConfigEntry[]> {
    const res = await fetch(`${API_BASE}/config`);
    if (!res.ok) throw new Error('Failed to fetch config');
    return await res.json();
  },

  async updateConfig(entries: ConfigEntryUpdate[]): Promise<ConfigEntry[]> {
    const res = await fetch(`${API_BASE}/config`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ entries })
    });
    if (!res.ok) throw new Error('Failed to update config');
    const data = await res.json();
    return data.updated || data;
  },

  async searchRegion(regionId: string): Promise<SearchRegionResponse> {
    const summariesRes = await fetch(`${API_BASE}/regions/${regionId}/summaries`);
    if (summariesRes.ok) {
      const data = await summariesRes.json();
      if (data.summaries.length > 0) {
        return { status: 'DONE', summaries: data.summaries };
      }
    }
    
    const generateRes = await fetch(`${API_BASE}/regions/${regionId}/generate`, {
      method: 'POST'
    });
    if (!generateRes.ok) throw new Error('Failed to generate region summary');
    
    return { status: 'PROCESSING', summaries: [] };
  },

  async generateSummary(regionId: string): Promise<GenerateSummaryResult> {
    const res = await fetch(`${API_BASE}/regions/${regionId}/generate`, {
      method: 'POST'
    });
    if (!res.ok) throw new Error('Failed to generate summary');
    return await res.json();
  },

  async getSummaries(regionId: string): Promise<SearchRegionResponse> {
    const res = await fetch(`${API_BASE}/regions/${regionId}/summaries`);
    if (!res.ok) throw new Error('Failed to get summaries');
    return await res.json();
  },

  async getBatchStatus(batchId: string): Promise<BatchStatusResult> {
    const res = await fetch(`${API_BASE}/batches/${batchId}/status`);
    if (!res.ok) throw new Error('Failed to get batch status');
    return await res.json();
  },

  async getRegionStatus(regionId: string): Promise<RegionStatusResult> {
    const res = await fetch(`${API_BASE}/regions/${regionId}/status`);
    if (!res.ok) throw new Error('Failed to get region status');
    return await res.json();
  },

  async getChunkSource(chunkId: string): Promise<ChunkSourceResponse> {
    const res = await fetch(`${API_BASE}/chunks/${chunkId}/source`);
    if (!res.ok) throw new Error('Failed to get chunk source');
    return await res.json();
  },

  async getPipelineStats(): Promise<PipelineStats> {
    const res = await fetch(`${API_BASE}/pipeline/stats`);
    if (!res.ok) throw new Error('Failed to fetch pipeline stats');
    return await res.json();
  },

  async listBrainRegions(): Promise<RegionMapping[]> {
    const res = await fetch(`${API_BASE}/regions`);
    if (!res.ok) throw new Error('Failed to list brain regions');
    return await res.json();
  },

  async getWorkerStatus(): Promise<WorkerStatus[]> {
    const res = await fetch(`${API_BASE}/workers/status`);
    if (!res.ok) throw new Error('Failed to get worker status');
    return await res.json();
  },

  async allocateWorkers(count: number): Promise<WorkerAllocationResponse> {
    const res = await fetch(`${API_BASE}/workers/allocate`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        worker_count: count,
        task_timeout_secs: 300,
        max_retry_attempts: 3
      })
    });
    if (!res.ok) throw new Error('Failed to allocate workers');
    return await res.json();
  },

  async stopWorker(workerId: string): Promise<WorkerStopResponse> {
    const res = await fetch(`${API_BASE}/workers/stop`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ worker_ids: [workerId] })
    });
    if (!res.ok) throw new Error('Failed to stop worker');
    return await res.json();
  },

  async stopAllWorkers(): Promise<WorkerStopResponse> {
    const res = await fetch(`${API_BASE}/workers/stop`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ worker_ids: [] })
    });
    if (!res.ok) throw new Error('Failed to stop all workers');
    return await res.json();
  }
};

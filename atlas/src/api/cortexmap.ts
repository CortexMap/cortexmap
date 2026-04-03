import axios from 'axios';
import type { CortexmapRegion, RegionStatus, RegionSummary } from '../types';

const API_BASE = import.meta.env.VITE_API_BASE_URL || 'https://capstone.ssdd.dev/orch/api';

const api = axios.create({ baseURL: API_BASE, timeout: 15000 });

export async function fetchAllRegions(): Promise<CortexmapRegion[]> {
  const { data } = await api.get<CortexmapRegion[]>('/regions');
  return data;
}

export async function fetchRegionStatus(uuid: string): Promise<RegionStatus> {
  const { data } = await api.get<RegionStatus>(`/regions/${uuid}/status`);
  return data;
}

export async function fetchRegionSummaries(uuid: string): Promise<RegionSummary[]> {
  const { data } = await api.get<{ summaries: RegionSummary[] }>(`/regions/${uuid}/summaries`);
  return data.summaries || [];
}

export async function generateSummary(uuid: string): Promise<unknown> {
  const { data } = await api.post(`/regions/${uuid}/generate`);
  return data;
}

export async function fetchBatchStatus(batchId: string): Promise<unknown> {
  const { data } = await api.get(`/batches/${batchId}/status`);
  return data;
}

/** Fetch source metadata for a chunk ID. Returns pmc_id, uid, query. */
export async function fetchChunkSource(chunkId: string): Promise<{
  source_pmc_id: string | null;
  source_uid: string | null;
  source_query: string | null;
} | null> {
  try {
    const { data } = await api.get(`/chunks/${chunkId}/source`);
    return data;
  } catch {
    return null;
  }
}

import type { BrainRegion } from '../types';

// Empty = same origin (Vite proxies /api to BFF in dev). Set VITE_API_URL for production.
const API_BASE = (import.meta.env.VITE_API_URL ?? '').replace(/\/$/, '');

function apiUrl(path: string, search = ''): string {
  const full = path.startsWith('/api') ? path : `/api/${path}`;
  return API_BASE ? `${API_BASE}${full}${search}` : `${full}${search}`;
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error((err as { error?: string }).error || `HTTP ${res.status}`);
  }
  return res.json();
}

/**
 * Fetch all brain regions from the BFF API.
 */
export async function fetchAllBrainRegions(): Promise<BrainRegion[]> {
  const res = await fetch(apiUrl('brain-regions'));
  return handleResponse<BrainRegion[]>(res);
}

/**
 * Search brain regions by query term.
 */
export async function searchBrainRegions(query: string): Promise<BrainRegion[]> {
  const q = encodeURIComponent(query.trim());
  const res = await fetch(apiUrl('brain-regions', `?q=${q}`));
  return handleResponse<BrainRegion[]>(res);
}

/**
 * Fetch brain regions - either all (when query is empty) or filtered by search.
 */
export async function fetchBrainRegions(query: string): Promise<BrainRegion[]> {
  if (!query.trim()) {
    return fetchAllBrainRegions();
  }
  return searchBrainRegions(query);
}

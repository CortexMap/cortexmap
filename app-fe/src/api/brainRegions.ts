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
 * NOTE: Using demo endpoint for now (no database required).
 * Change 'brain-regions/demo' to 'brain-regions' when database is available.
 */
export async function fetchAllBrainRegions(): Promise<BrainRegion[]> {
  const res = await fetch(apiUrl('brain-regions/demo'));
  return handleResponse<BrainRegion[]>(res);
}

/**
 * Search brain regions by query term.
 * NOTE: Demo data doesn't support search yet, returns all regions.
 * Update to use 'brain-regions?q=' when database is available.
 */
export async function searchBrainRegions(query: string): Promise<BrainRegion[]> {
  // TODO: For now, demo endpoint doesn't support search
  // Will filter client-side
  const res = await fetch(apiUrl('brain-regions/demo'));
  const all = await handleResponse<BrainRegion[]>(res);
  
  // Client-side filtering for demo
  const q = query.trim().toLowerCase();
  if (!q) return all;
  
  return all.filter(region => 
    region.name.toLowerCase().includes(q) ||
    region.location.lobe.toLowerCase().includes(q) ||
    region.location.anatomical_region.toLowerCase().includes(q) ||
    region.function_diseases.function_description.toLowerCase().includes(q) ||
    region.function_diseases.disease_description.toLowerCase().includes(q)
  );
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

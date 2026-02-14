import { QueryHistory } from '../types/enhanced';

const STORAGE_KEY = 'cortexmap_query_history';

export const queryHistoryService = {
  getAll(): QueryHistory[] {
    const data = localStorage.getItem(STORAGE_KEY);
    return data ? JSON.parse(data) : [];
  },

  add(query: QueryHistory): void {
    const history = this.getAll();
    
    // Check if query already exists
    const existingIndex = history.findIndex(q => q.query.toLowerCase() === query.query.toLowerCase());
    
    if (existingIndex !== -1) {
      // Update existing query
      history[existingIndex] = { ...history[existingIndex], ...query, timestamp: Date.now() };
    } else {
      // Add new query at the beginning
      history.unshift(query);
    }
    
    localStorage.setItem(STORAGE_KEY, JSON.stringify(history.slice(0, 50))); // Keep last 50
  },

  update(id: string, updates: Partial<QueryHistory>): void {
    const history = this.getAll();
    const index = history.findIndex(q => q.id === id);
    if (index !== -1) {
      history[index] = { ...history[index], ...updates };
      localStorage.setItem(STORAGE_KEY, JSON.stringify(history));
    }
  },

  delete(id: string): void {
    const history = this.getAll().filter(q => q.id !== id);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(history));
  },

  clear(): void {
    localStorage.removeItem(STORAGE_KEY);
  }
};

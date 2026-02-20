import { ChatMessage } from '../types';

const STORAGE_KEY = 'brainatlas_chat_history';
const MAX_MESSAGES = 100; // Limit to prevent localStorage bloat

export const chatStorage = {
  save(messages: ChatMessage[]) {
    // Keep only the last 100 messages
    const trimmed = messages.slice(-MAX_MESSAGES);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(trimmed));
  },

  load(): ChatMessage[] {
    try {
      const data = localStorage.getItem(STORAGE_KEY);
      return data ? JSON.parse(data) : [];
    } catch (err) {
      console.error('Failed to load chat history:', err);
      return [];
    }
  },

  clear() {
    localStorage.removeItem(STORAGE_KEY);
  }
};

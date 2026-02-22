// API Configuration
// Change the base URL here to point to your backend server

// Option 1: Use environment variable (recommended for production)
// Create a .env file in the root with: VITE_API_BASE_URL=http://your-server:port/orch/api
export const API_BASE_URL = import.meta.env.VITE_API_BASE_URL || 'https://capstone.ssdd.dev/orch/api';

// Debug logging configuration
export const DEBUG_MODE = import.meta.env.VITE_DEBUG === 'true' || true; // Set to false to disable logs

// Logging utility
export const logger = {
  info: (...args) => {
    if (DEBUG_MODE) {
      console.log('[INFO]', new Date().toISOString(), ...args);
    }
  },
  error: (...args) => {
    console.error('[ERROR]', new Date().toISOString(), ...args);
  },
  warn: (...args) => {
    if (DEBUG_MODE) {
      console.warn('[WARN]', new Date().toISOString(), ...args);
    }
  },
  debug: (...args) => {
    if (DEBUG_MODE) {
      console.debug('[DEBUG]', new Date().toISOString(), ...args);
    }
  },
  api: (method, url, data = null) => {
    if (DEBUG_MODE) {
      console.log(
        `%c[API ${method}]`,
        'color: #3b82f6; font-weight: bold',
        url,
        data ? data : ''
      );
    }
  },
  apiSuccess: (method, url, response) => {
    if (DEBUG_MODE) {
      console.log(
        `%c[API ${method} SUCCESS]`,
        'color: #10b981; font-weight: bold',
        url,
        response
      );
    }
  },
  apiError: (method, url, error) => {
    const errorDetails = {
      message: error.message,
      status: error.response?.status,
      statusText: error.response?.statusText,
      data: error.response?.data,
      config: {
        url: error.config?.url,
        method: error.config?.method,
        baseURL: error.config?.baseURL,
        headers: error.config?.headers,
      }
    };
    
    console.error(
      `%c[API ${method} ERROR]`,
      'color: #ef4444; font-weight: bold',
      url,
      errorDetails
    );
  }
};

// Log configuration on load
logger.info('Configuration loaded:', {
  API_BASE_URL,
  DEBUG_MODE,
  environment: import.meta.env.MODE
});

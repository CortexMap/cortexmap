# Brain Atlas Frontend

A visually rich React application for exploring brain regions and viewing AI-generated summaries.

## Features

- **Browse Brain Regions**: View all available brain regions with color coding and metadata
- **Search & Filter**: Search by name or acronym, sort by various criteria
- **Region Details**: View detailed information about each brain region
- **Summary Generation**: Generate AI-powered summaries for brain regions
- **Progress Tracking**: Real-time status updates for summary generation
- **Pipeline Statistics**: Dashboard showing overall system processing status
- **Comprehensive Logging**: Detailed console logs for debugging API connections

## Tech Stack

- **React 18** - UI framework
- **Vite** - Build tool and dev server
- **Axios** - HTTP client
- **Recharts** - Data visualization
- **Lucide React** - Icon library

## Getting Started

### Prerequisites

- Node.js 16+ and npm
- Backend orchestrator service running (default: `http://localhost:8080`)

### Installation

```bash
npm install
```

### Configuration

**To change the backend URL**, edit `src/config.js`:

```javascript
export const API_BASE_URL = 'http://your-server:8080/orch/api';
```

### Development

```bash
npm run dev
```

The application will be available at `http://localhost:3000`

### Build for Production

```bash
npm run build
```

## API Endpoints Used

- `GET /orch/api/regions` - Fetch all brain regions
- `GET /orch/api/regions/{id}/status` - Get region processing status
- `GET /orch/api/regions/{id}/summaries` - Fetch summaries for a region
- `POST /orch/api/regions/{id}/generate` - Trigger new summary generation
- `GET /orch/api/batches/{id}/status` - Get batch processing status
- `GET /orch/api/pipeline/stats` - Get pipeline statistics
- `GET /orch/api/workers/status` - Get worker status
- `POST /orch/api/workers/allocate` - Allocate new workers
- `POST /orch/api/workers/stop` - Stop workers

## Batch Tracking & Persistence

When you trigger a summary generation, the application:
1. Receives a `batch_id` from the backend
2. Stores it in a browser cookie (expires in 24 hours)
3. Polls `/batches/{id}/status` every 2 seconds for updates
4. Shows progress in the "Processing Status" card
5. Automatically removes the batch ID when complete

**Benefits:**
- Batch progress survives page refreshes
- Multiple regions can have active batches simultaneously
- Generate button stays disabled during active generation
- Cookie is automatically cleaned up after 24 hours

**Cookie Format:**
```json
{
  "brainatlas_batch_ids": {
    "region-uuid-1": "batch-uuid-1",
    "region-uuid-2": "batch-uuid-2"
  }
}
```

## Debugging

The application includes comprehensive logging. Open browser console (F12) to see:

- **API requests** - All HTTP requests with URLs and data
- **API responses** - Success/failure with full response details
- **Error details** - Network errors, server errors, etc.
- **Component lifecycle** - When components mount/unmount
- **Auto-refresh** - When background polling occurs

### Common Issues

**"Failed to fetch regions"** error:
1. Check browser console for detailed error information
2. Verify backend is running: `curl http://localhost:8080/orch/health`
3. Check that URL in `src/config.js` matches your backend
4. Look for CORS errors in console

**Network Error**:
- Backend server is not running or not accessible
- Check firewall settings
- Verify the port number is correct

**404 Not Found**:
- API endpoint path is incorrect
- Backend routes may be different than expected

# CortexMap Paper Fetcher - Frontend

A real-time web application for searching and fetching PubMed papers with live status tracking, connected to the CortexMap backend.

## Features

- **Real-time Status Updates**: Poll every 200ms to track the fetching progress of summary, abstract, and PDF for each paper
- **Visual Feedback**: See checkmarks (✓) for successful fetches and crosses (✗) for failures
- **Smart Retry Logic**: Backend automatically retries failed components up to 3 times
- **Worker Management**: Allocate and stop worker processes from the UI
- **Queue Statistics**: Real-time stats showing pending, in-progress, completed, and failed tasks
- **Modern UI**: Beautiful gradient design with smooth animations and transitions
- **Component Tracking**: Track the status of each paper component (summary, abstract, PDF) independently
- **S3 Integration**: Fetched content is stored in S3 with keys displayed in the UI

## Architecture

```
Frontend (React + TypeScript)
    ↓ HTTP REST API (proxied through Vite)
Backend (Rust gRPC Server on :50051)
    ↓
Task Queue (PostgreSQL) + Workers + S3 Storage
```

## Getting Started

### Prerequisites

- Node.js 18+ and npm
- Backend server running on `localhost:8080` (REST API wrapper for gRPC)

### Installation

```bash
npm install
```

### Development

```bash
npm start
```

This will start the development server at `http://localhost:3000` with automatic proxy to the backend API.

### Build

```bash
npm run build
```

## Usage

1. **Start Workers**: Allocate worker processes to begin fetching papers
2. **Search**: Enter a PubMed query (e.g., "alzheimer's disease", "COVID-19")
3. **Track**: Watch real-time status updates with visual indicators:
   - ⏳ **Pending**: Waiting to be processed
   - 🔄 **In Progress**: Currently being fetched (with retry count if > 0)
   - ✓ **Completed**: Successfully fetched and stored in S3
   - ✗ **Failed**: Failed after maximum retry attempts
4. **Monitor**: View queue statistics and S3 keys for completed components
5. **Stop Workers**: Stop all workers when done

## API Integration

The frontend communicates with the backend through a REST API wrapper:

### Endpoints

- `POST /api/enqueue` - Enqueue a query for fetching
- `GET /api/queue/status` - Get overall queue statistics
- `GET /api/task/:pmcId` - Get detailed status for a specific task
- `POST /api/tasks/batch` - Get status for multiple tasks (batch)
- `POST /api/workers/allocate` - Start worker processes
- `POST /api/workers/stop` - Stop worker processes
- `GET /api/workers/status` - Get worker status information

## Configuration

### Vite Dev Server

The Vite development server is configured to proxy API requests to the backend:

```typescript
server: {
  proxy: {
    '/api': {
      target: 'http://localhost:8080',
      changeOrigin: true,
      rewrite: (path) => path.replace(/^\/api/, '')
    }
  }
}
```

### Backend Connection

Update `vite.config.ts` if your backend runs on a different port.

## Technical Details

- **Framework**: React 18 with TypeScript
- **Build Tool**: Vite 5
- **Polling Interval**: 200ms
- **Max Retries**: 3 attempts per component (configured in backend)
- **Components**: Summary, Abstract, PDF

## Project Structure

```
fetcher_fe/
├── proto/
│   └── queue.proto          # gRPC protocol definition (for reference)
├── src/
│   ├── api/
│   │   └── backendApi.ts    # Backend API client
│   ├── components/
│   │   ├── PaperCard.tsx    # Individual paper display with status
│   │   ├── PaperList.tsx    # List of papers with queue stats
│   │   ├── SearchBar.tsx    # Search input
│   │   └── StatusIndicator.tsx # Status icons and labels
│   ├── hooks/
│   │   └── usePaperFetcher.ts # Main fetching logic with polling
│   ├── types.ts             # TypeScript interfaces
│   ├── App.tsx              # Main application with worker controls
│   └── main.tsx             # Entry point
└── package.json
```

## Backend Requirements

The backend must provide a REST API wrapper for the gRPC service defined in `queue.proto`. The expected response formats match the proto definitions:

- **EnqueueResponse**: `{ success, tasksEnqueued, pmcIds[], errorMessage }`
- **StatusResponse**: `{ totalTasks, pendingTasks, inProgressTasks, completedTasks, failedTasks, activeWorkers }`
- **TaskDetailsResponse**: `{ found, pmcId, status, components[], errorMessage }`
- **ComponentStatus**: `{ componentType, status, attemptCount, maxAttempts, s3Key, errorMessage }`

## Development

### Running with Backend

1. Start the backend server (default port: 50051 for gRPC, 8080 for REST wrapper)
2. Run `npm start` in this directory
3. Navigate to `http://localhost:3000`

### Without Backend (Mock Mode)

To test the UI without the backend, you can modify `src/api/backendApi.ts` to return mock data.

## Troubleshooting

### Connection Issues

- Ensure the backend is running and accessible at `localhost:8080`
- Check browser console for CORS or network errors
- Verify Vite proxy configuration in `vite.config.ts`

### Polling Not Working

- Check that PMC IDs are being returned from the enqueue endpoint
- Verify the backend implements the batch task details endpoint
- Check browser console for API errors

## Future Enhancements

- WebSocket connection for real-time push updates (instead of polling)
- Download fetched content directly from S3
- Advanced search filters and pagination
- Export results to various formats
- Persistent storage of search history
- Batch operations and bulk management

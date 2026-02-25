# BrainAtlas Frontend

> React-based user interface for exploring brain regions with AI-powered summaries

A modern, responsive web application for neuroscientists to explore brain atlas data, view AI-generated summaries, and track processing pipelines in real-time.

## Features

- **Brain Region Explorer**: Browse and search 1000+ brain regions from Allen Brain Atlas
- **AI Summary Viewer**: Read comprehensive, citation-backed summaries with markdown rendering
- **Pipeline Dashboard**: Monitor processing status across all regions with real-time updates
- **Worker Management**: Control fetcher workers to scale paper processing
- **Batch Tracking**: Track summary generation progress with persistent cookie storage
- **Responsive Design**: Clean, modern UI with dark theme optimized for readability

## Architecture

### Component Hierarchy

```
App (Main Container)
├── WorkerManagement (Collapsible Panel)
│   ├── Worker Controls (Allocate/Stop)
│   └── Worker Cards (Status, Metrics)
├── PipelineStats (Collapsible Panel)
│   ├── Status Distribution Chart
│   └── Completion Statistics
├── RegionList (Grid View)
│   └── RegionCard[] (Region Previews)
└── RegionDetail (Detail View)
    ├── Region Header (Name, Color, Acronym)
    ├── Processing Status Card
    ├── Batch Progress (Real-time Polling)
    └── SummaryDisplay (Markdown Rendering)
```

### State Management

**Local Component State** (no global state management):
- Each component manages its own data fetching
- Auto-refresh intervals for real-time updates
- Cookie-based persistence for batch tracking

**Data Flow**:
```
User Action → API Call (Axios) → Backend Response → State Update → UI Render
```

## Getting Started

### Prerequisites

- Node.js 22+ (LTS recommended)
- npm 10+
- Backend services running (orch, brainatlas-be, fetcher-be)

### Development Setup

1. **Install dependencies**:
   ```bash
   npm install
   ```

2. **Configure API endpoint**:
   
   Edit `src/config.js` or create `.env`:
   ```bash
   VITE_API_BASE_URL=http://localhost:8082/orch/api
   VITE_DEBUG=true
   ```

3. **Start development server**:
   ```bash
   npm run dev
   ```

4. **Open browser**:
   Navigate to `http://localhost:5173`

### Production Build

```bash
# Build optimized bundle
npm run build

# Preview production build
npm run preview
```

## Component Overview

### `App.jsx`

Main application container managing routing between list and detail views.

**Key Features**:
- Region list fetching and error handling
- Navigation state management
- Global error display banner
- Structured logging integration

**API Calls**:
- `GET /regions` - Fetch all brain regions on mount

### `RegionList.jsx`

Grid view displaying all available brain regions.

**Features**:
- Color-coded region cards
- Search/filter capabilities
- Click-to-navigate to detail view

### `RegionDetail.jsx`

Detailed view for individual brain regions.

**Key Features**:
- Region metadata display (UUID, parent region, structure order)
- Processing status badge with real-time updates
- Batch progress tracking with cookie persistence
- Summary generation triggers
- Auto-refresh during active processing (3s interval)

**Auto-Refresh Logic**:
```javascript
// Auto-refreshes when status is in-progress
if (['FetchQueued', 'Fetching', 'LlmQueued', 'Processing'].includes(status)) {
  const interval = setInterval(() => {
    fetchRegionStatus();
    fetchRegionSummaries();
  }, 3000);
}
```

**Cookie Management**:
- Stores batch IDs per region in `brainatlas_batch_ids` cookie
- Survives page refreshes and browser restarts (24h expiry)
- Automatically cleared when batch completes

**API Calls**:
- `GET /regions/{id}/status` - Get processing status
- `GET /regions/{id}/summaries` - Fetch summaries
- `POST /regions/{id}/generate` - Start summary generation
- `GET /batches/{batch_id}/status` - Poll batch progress

### `SummaryDisplay.jsx`

Renders markdown summaries with accordion-style collapsible sections.

**Features**:
- Markdown rendering via `react-markdown`
- Multiple summary versions (historical tracking)
- Expandable/collapsible summary cards
- Timestamps for creation dates

### `PipelineStats.jsx`

Dashboard showing system-wide processing statistics.

**Features**:
- Collapsible panel to save screen space
- Bar chart visualization (Recharts)
- Auto-refresh every 10 seconds
- Completion rate percentage
- Status breakdown:
  - Done, Processing, LLM Queued
  - Fetching, Fetch Queued
  - Not Started, Failed, Invalidated

**API Calls**:
- `GET /pipeline/stats`

### `WorkerManagement.jsx`

Control panel for managing fetcher workers.

**Features**:
- Allocate workers with configurable count
- Stop individual workers or all workers
- Real-time worker status (active, idle, busy)
- Worker metrics:
  - Current task
  - Tasks completed
  - Uptime
- Auto-refresh every 5 seconds

**API Calls**:
- `GET /workers/status`
- `POST /workers/allocate`
- `POST /workers/stop`

## Configuration

### `config.js`

Central configuration file for API and logging.

```javascript
// API Configuration
export const API_BASE_URL = import.meta.env.VITE_API_BASE_URL 
  || 'https://capstone.ssdd.dev/orch/api';

// Debug Mode
export const DEBUG_MODE = import.meta.env.VITE_DEBUG === 'true' || true;
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `VITE_API_BASE_URL` | Backend API base URL | `https://capstone.ssdd.dev/orch/api` |
| `VITE_DEBUG` | Enable debug logging | `false` |

**Important**: Vite environment variables are **baked into the build** at compile time, not runtime.

### Logger Utility

Structured logging with color-coded output:

```javascript
logger.info('Application started');
logger.error('API call failed', error);
logger.api('GET', '/regions');
logger.apiSuccess('GET', '/regions', response);
logger.apiError('POST', '/generate', error);
```

Log levels:
- `info` - General information (only in DEBUG mode)
- `error` - Always logged
- `warn` - Warnings (only in DEBUG mode)
- `debug` - Detailed debugging (only in DEBUG mode)
- `api` - API call tracking with color coding

## Styling

### CSS Architecture

Each component has its own CSS file:
```
src/
├── App.css                      # Global styles
├── components/
│   ├── RegionList.css
│   ├── RegionDetail.css
│   ├── SummaryDisplay.css
│   ├── PipelineStats.css
│   └── WorkerManagement.css
└── index.css                    # Base styles, CSS variables
```

### Color Scheme

Dark theme with vibrant accents:

```css
:root {
  --primary-bg: #0f172a;        /* Slate 900 */
  --secondary-bg: #1e293b;      /* Slate 800 */
  --accent-blue: #3b82f6;       /* Blue 500 */
  --accent-purple: #8b5cf6;     /* Violet 500 */
  --success: #10b981;           /* Green 500 */
  --error: #ef4444;             /* Red 500 */
  --warning: #f59e0b;           /* Amber 500 */
  --text-primary: #f1f5f9;      /* Slate 100 */
  --text-secondary: #cbd5e1;    /* Slate 300 */
}
```

## API Integration

### Axios Configuration

Base Axios instance automatically configured via `config.js`:

```javascript
import axios from 'axios';
import { API_BASE_URL } from './config';

// All requests use API_BASE_URL as prefix
const response = await axios.get(`${API_BASE_URL}/regions`);
```

### Error Handling Pattern

Consistent error handling across components:

```javascript
try {
  setLoading(true);
  setError(null);
  
  const response = await axios.get(url);
  logger.apiSuccess('GET', url, response.data);
  
  setData(response.data);
} catch (err) {
  logger.apiError('GET', url, err);
  
  let errorMessage = 'Request failed. ';
  if (err.code === 'ERR_NETWORK') {
    errorMessage += `Cannot connect to ${API_BASE_URL}`;
  } else if (err.response) {
    errorMessage += `Server error: ${err.response.status}`;
  }
  
  setError(errorMessage);
} finally {
  setLoading(false);
}
```

### CORS Considerations

Frontend makes cross-origin requests to backend. Ensure backend CORS is configured:

```env
# Backend .env
CORS_ORIGIN=http://localhost:5173  # Development
# CORS_ORIGIN=https://your-domain.com  # Production
```

## Testing

### Development Testing

1. **Start backend services**:
   ```bash
   cd .. && ./start-services.sh
   ```

2. **Verify API connectivity**:
   ```bash
   curl http://localhost:8082/orch/health
   ```

3. **Start frontend**:
   ```bash
   npm run dev
   ```

4. **Test scenarios**:
   - Browse regions list
   - Click into region detail
   - Start summary generation
   - Monitor batch progress
   - Check worker management
   - Verify auto-refresh behavior

### Build Testing

```bash
# Build production bundle
npm run build

# Check bundle size
ls -lh dist/

# Preview production build
npm run preview
```

### Linting

```bash
# Run ESLint
npm run lint

# Auto-fix issues
npm run lint -- --fix
```

## Troubleshooting

### Cannot Connect to Backend

**Symptoms**: "Network Error" on all API calls

**Solutions**:
1. Verify backend is running: `curl http://localhost:8082/orch/health`
2. Check `VITE_API_BASE_URL` in config.js
3. Inspect browser console for CORS errors
4. Verify backend `CORS_ORIGIN` setting

### Summaries Not Displaying

**Symptoms**: Region detail shows "No Summaries Available" despite processing complete

**Checks**:
1. Open browser DevTools -> Network tab
2. Check `/regions/{id}/summaries` response
3. Verify response structure: `{ summaries: [...] }`
4. Check console for JavaScript errors

**Common Causes**:
- API response format changed
- Region ID mismatch (UUID vs integer)
- Backend returned error but status 200

### Auto-Refresh Not Working

**Symptoms**: Status doesn't update automatically

Verify:
- Status is in correct state for auto-refresh
- Interval is being created and cleared properly
- Component hasn't unmounted

### Batch Progress Not Persisting

**Symptoms**: Batch progress lost on page refresh

**Checks**:
1. Open DevTools -> Application -> Cookies
2. Check for `brainatlas_batch_ids` cookie
3. Verify JSON structure: `{"region_id": "batch_id"}`

## Deployment

### Docker Build

Multi-stage Dockerfile for optimized production builds:

```bash
# Build with API URL baked in
docker build \
  --build-arg VITE_API_BASE_URL=https://api.example.com/orch/api \
  -f Dockerfile \
  -t brainatlas-fe:latest .

# Run with nginx
docker run -p 80:80 brainatlas-fe:latest
```

### Nginx Configuration

Included `nginx.conf` handles SPA routing:

```nginx
server {
  listen 80;
  root /usr/share/nginx/html;
  index index.html;

  # SPA fallback - all routes serve index.html
  location / {
    try_files $uri $uri/ /index.html;
  }

  # Cache static assets
  location ~* \.(js|css|png|jpg|jpeg|gif|ico|svg)$ {
    expires 1y;
    add_header Cache-Control "public, immutable";
  }
}
```

### Environment-Specific Builds

**Development**:
```bash
VITE_API_BASE_URL=http://localhost:8082/orch/api npm run build
```

**Staging**:
```bash
VITE_API_BASE_URL=https://staging-api.example.com/orch/api npm run build
```

**Production**:
```bash
VITE_API_BASE_URL=https://api.example.com/orch/api npm run build
```

## Technology Stack

| Technology | Version | Purpose |
|------------|---------|---------|
| React | 19.2.0 | UI framework |
| Vite | 7.3.1 | Build tool & dev server |
| Axios | 1.13.5 | HTTP client |
| Recharts | 3.7.0 | Data visualization |
| react-markdown | 10.1.0 | Markdown rendering |
| lucide-react | 0.575.0 | Icon library |
| js-cookie | 3.0.5 | Cookie management |

---

**BrainAtlas Frontend** - Intuitive exploration of AI-powered neuroscience research

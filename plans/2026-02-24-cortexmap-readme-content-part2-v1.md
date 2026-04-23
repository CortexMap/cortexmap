# CortexMap README Content - Part 2

Continuation of README documentation for remaining services.

---

## BrainAtlas Frontend README (`/brainatlas-fe/README.md`)

```markdown
# BrainAtlas Frontend

> React-based user interface for exploring brain regions with AI-powered summaries

A modern, responsive web application for neuroscientists to explore brain atlas data, view AI-generated summaries, and track processing pipelines in real-time.

## 🎯 Features

- **Brain Region Explorer**: Browse and search 1000+ brain regions from Allen Brain Atlas
- **AI Summary Viewer**: Read comprehensive, citation-backed summaries with markdown rendering
- **Pipeline Dashboard**: Monitor processing status across all regions with real-time updates
- **Worker Management**: Control fetcher workers to scale paper processing
- **Batch Tracking**: Track summary generation progress with persistent cookie storage
- **Responsive Design**: Clean, modern UI with dark theme optimized for readability

## 🏗️ Architecture

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

## 🚀 Getting Started

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

## 📊 Component Overview

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

## ⚙️ Configuration

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

## 🎨 Styling

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

### Responsive Design

Breakpoints:
- Desktop: 1024px+
- Tablet: 768px - 1023px
- Mobile: < 768px

Grid layouts adapt:
```css
.regions-grid {
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
}

@media (max-width: 768px) {
  .regions-grid {
    grid-template-columns: 1fr;
  }
}
```

## 🔄 API Integration

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

## 🧪 Testing

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

## 🐛 Troubleshooting

### Cannot Connect to Backend

**Symptoms**: "Network Error" on all API calls

**Solutions**:
1. Verify backend is running: `curl http://localhost:8082/orch/health`
2. Check `VITE_API_BASE_URL` in config.js
3. Inspect browser console for CORS errors
4. Verify backend `CORS_ORIGIN` setting

**Debug**:
```javascript
// Add to config.js temporarily
console.log('API_BASE_URL:', API_BASE_URL);
console.log('Environment:', import.meta.env.MODE);
```

### Summaries Not Displaying

**Symptoms**: Region detail shows "No Summaries Available" despite processing complete

**Checks**:
1. Open browser DevTools → Network tab
2. Check `/regions/{id}/summaries` response
3. Verify response structure: `{ summaries: [...] }`
4. Check console for JavaScript errors

**Common Causes**:
- API response format changed
- Region ID mismatch (UUID vs integer)
- Backend returned error but status 200

### Auto-Refresh Not Working

**Symptoms**: Status doesn't update automatically

**Checks**:
```javascript
// Add to RegionDetail.jsx temporarily
useEffect(() => {
  console.log('Auto-refresh active:', status);
}, [status]);
```

Verify:
- Status is in correct state for auto-refresh
- Interval is being created and cleared properly
- Component hasn't unmounted

### Batch Progress Not Persisting

**Symptoms**: Batch progress lost on page refresh

**Checks**:
1. Open DevTools → Application → Cookies
2. Check for `brainatlas_batch_ids` cookie
3. Verify JSON structure: `{"region_id": "batch_id"}`

**Debug**:
```javascript
// Check cookie storage
import Cookies from 'js-cookie';
console.log('Batch cookies:', Cookies.get('brainatlas_batch_ids'));
```

## 🚀 Deployment

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

### Health Check Endpoint

Add to nginx.conf for load balancer health checks:

```nginx
location /health {
  access_log off;
  return 200 "healthy\n";
  add_header Content-Type text/plain;
}
```

## 📈 Performance Optimization

### Bundle Size Analysis

```bash
# Install analyzer
npm install --save-dev rollup-plugin-visualizer

# Add to vite.config.js
import { visualizer } from 'rollup-plugin-visualizer';

export default {
  plugins: [
    react(),
    visualizer({ open: true })
  ]
}

# Build and analyze
npm run build
```

### Code Splitting

Vite automatically code-splits routes and dynamic imports:

```javascript
// Lazy load heavy components
const RegionDetail = lazy(() => import('./components/RegionDetail'));

<Suspense fallback={<Loading />}>
  <RegionDetail region={selected} />
</Suspense>
```

### Image Optimization

If adding images:
```javascript
// Use Vite's asset handling
import logo from './assets/logo.png?format=webp&w=200';
```

## 🔐 Security Considerations

### XSS Prevention

- React auto-escapes JSX content
- `react-markdown` sanitizes HTML by default
- No `dangerouslySetInnerHTML` usage

### API Key Security

- Never commit API keys to repository
- Use environment variables for sensitive config
- Backend handles all API keys (OpenRouter, etc.)

### Content Security Policy

Consider adding CSP headers in nginx:

```nginx
add_header Content-Security-Policy 
  "default-src 'self'; 
   script-src 'self' 'unsafe-inline'; 
   style-src 'self' 'unsafe-inline'; 
   connect-src 'self' https://api.example.com;";
```

## 🤝 Contributing

When contributing to brainatlas-fe:

1. Follow React hooks best practices
2. Maintain component-level CSS files
3. Use structured logging via `logger` utility
4. Handle loading and error states consistently
5. Add PropTypes or TypeScript for type safety
6. Run `npm run lint` before committing

### Code Style

- **Components**: PascalCase (RegionDetail.jsx)
- **Utilities**: camelCase (config.js, logger)
- **CSS**: BEM-style naming (region-detail__header)
- **Indentation**: 2 spaces
- **Quotes**: Single quotes for JSX, imports

## 📚 Technology Stack

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
```

---

## Fetcher Backend README (`/fetcher-be/README.md`)

```markdown
# Fetcher Backend

> High-performance PubMed paper fetching service with distributed worker pool architecture

The Fetcher Backend is a Rust-based service that manages a sophisticated task queue for downloading academic papers from PubMed Central (PMC). It features a scalable worker pool, component-based processing with intelligent retry mechanisms, and S3 storage integration.

## 🎯 Purpose

This service:

1. **Accepts queries** for neuroscience papers from PubMed
2. **Enqueues tasks** with priority-based scheduling
3. **Manages workers** that process tasks concurrently
4. **Fetches components**: PDF, abstract, and AI-generated summary
5. **Stores results** in S3-compatible storage
6. **Handles failures** with sophisticated retry logic

## 🏗️ Architecture

### Distributed Task Queue System

```
┌─────────────────────────────────────────────────────────┐
│                 HTTP API (Axum)                         │
│  Enqueue, Status, Workers, Task Details                │
└──────────────────┬──────────────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────────────┐
│              WorkerManager                              │
│  Allocate, Stop, Monitor Workers                        │
└──────────┬─────────────────────┬────────────────────────┘
           │                     │
    ┌──────▼──────┐       ┌──────▼──────┐
    │  Worker 1   │  ...  │  Worker N   │
    │  (Tokio Task)│       │ (Tokio Task)│
    └──────┬──────┘       └──────┬──────┘
           │                     │
┌──────────▼─────────────────────▼────────────────────────┐
│           PostgreSQL Task Queue                         │
│  Tasks: pending → in_progress → completed/failed        │
│  Components: summary, abstract, pdf (independent retry) │
└─────────────────────────────────────────────────────────┘
           │
           │  Upload
           ▼
┌─────────────────────────────────────────────────────────┐
│                S3 Storage                               │
│  papers/{pmc_id}/{component}                            │
└─────────────────────────────────────────────────────────┘
```

### Crate Organization

| Crate | Purpose |
|-------|---------|
| **cortexmap-be** | Main server, HTTP API, worker management |
| **cortexmap-infra** | Infrastructure trait definitions |
| **std-infra** | Concrete implementations (PostgreSQL, S3, HTTP) |
| **cortexmap-fetcher** | PubMed fetching logic (metadata, PDF, abstracts) |
| **cortexmap-database** | Diesel models and schema |
| **cortexmap-cli** | Command-line tools for debugging |

## 🚀 Getting Started

### Prerequisites

- Rust 1.75+ (2024 edition)
- PostgreSQL 15+
- S3-compatible storage (AWS S3 or MinIO)
- Diesel CLI: `cargo install diesel_cli --no-default-features --features postgres`

### Installation

1. **Set up database**:
   ```bash
   # Create database
   createdb cortexmap
   
   # Run migrations
   cd fetcher-be
   diesel migration run
   ```

2. **Configure environment**:
   ```bash
   export DATABASE_URL="postgres://user:password@localhost/cortexmap"
   export S3_ENDPOINT="https://s3.amazonaws.com"
   export S3_ACCESS_KEY="your_access_key"
   export S3_SECRET_KEY="your_secret_key"
   export S3_BUCKET="cortexmap-papers"
   export FETCHER_HTTP_ADDR="0.0.0.0:8080"
   export RUST_LOG="info"
   ```

3. **Build and run**:
   ```bash
   cargo build --release
   cargo run --bin cortexmap-be
   ```

4. **Allocate workers**:
   ```bash
   curl -X POST http://localhost:8080/api/queue/workers/allocate \
     -H "Content-Type: application/json" \
     -d '{"worker_count": 2, "task_timeout_secs": 300, "max_retry_attempts": 3}'
   ```

## 📊 Database Schema

### Tables

#### `fetch_tasks`
Core task queue with priority scheduling and worker assignment.

```sql
CREATE TABLE fetch_tasks (
  id UUID PRIMARY KEY,
  pmc_id VARCHAR(50) NOT NULL,
  query TEXT NOT NULL,
  status VARCHAR(50) DEFAULT 'pending',
  priority INTEGER DEFAULT 5,
  
  -- Worker assignment
  worker_id UUID,
  worker_version VARCHAR(50),
  heartbeat_at TIMESTAMP,
  
  -- Timing
  started_at TIMESTAMP,
  completed_at TIMESTAMP,
  last_processed_at TIMESTAMP,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  -- Error tracking
  error_message TEXT,
  retry_count INTEGER DEFAULT 0,
  
  UNIQUE (pmc_id, query)
);

-- Critical index for queue polling
CREATE INDEX idx_fetch_tasks_queue 
  ON fetch_tasks (status, priority DESC, created_at ASC);

-- Worker task tracking
CREATE INDEX idx_fetch_tasks_worker 
  ON fetch_tasks (worker_id, status);
```

#### `fetch_task_components`
Component-level tracking with independent retry logic.

```sql
CREATE TABLE fetch_task_components (
  id UUID PRIMARY KEY,
  task_id UUID NOT NULL REFERENCES fetch_tasks(id) ON DELETE CASCADE,
  component_type VARCHAR(50) NOT NULL,  -- 'summary', 'abstract', 'pdf'
  status VARCHAR(50) DEFAULT 'pending',
  
  -- S3 storage
  s3_key TEXT,
  
  -- Retry management
  attempt_count INTEGER DEFAULT 0,
  max_attempts INTEGER DEFAULT 3,
  error_message TEXT,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE (task_id, component_type)
);

CREATE INDEX idx_components_task ON fetch_task_components(task_id);
CREATE INDEX idx_components_status ON fetch_task_components(status);
```

#### `fetch_task_logs`
Comprehensive event logging for debugging and auditing.

```sql
CREATE TABLE fetch_task_logs (
  id UUID PRIMARY KEY,
  task_id UUID NOT NULL REFERENCES fetch_tasks(id) ON DELETE CASCADE,
  component_type VARCHAR(50),
  log_level VARCHAR(20) NOT NULL,  -- 'info', 'warn', 'error'
  message TEXT NOT NULL,
  metadata JSONB,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_logs_task ON fetch_task_logs(task_id, created_at DESC);
CREATE INDEX idx_logs_level ON fetch_task_logs(log_level, created_at DESC);
```

## 🔄 Task Processing Flow

### 1. Enqueue Query

```
POST /api/queue/enqueue
    ↓
Query PubMed ESummary API
    ↓
For each PMC ID found:
    ├─ Check if task exists (pmc_id + query unique)
    ├─ Create fetch_task (status: pending)
    └─ Create 3 components: summary, abstract, pdf
```

### 2. Worker Claims Task

```
Worker polls: get_next_pending_task()
    ↓
PostgreSQL query with FOR UPDATE SKIP LOCKED
    ↓
If task found:
    ├─ Set status = 'in_progress'
    ├─ Set worker_id = <worker_uuid>
    ├─ Set heartbeat_at = NOW()
    └─ Return task to worker
```

### 3. Process Components

```
For each pending component:
    ├─ Increment attempt_count
    ├─ Set component status = 'in_progress'
    ├─ Fetch component:
    │   ├─ summary: PubMed abstract → S3
    │   ├─ abstract: Extract from XML → S3
    │   └─ pdf: Download PDF → S3
    ├─ On success:
    │   ├─ Set component status = 'completed'
    │   └─ Store s3_key
    └─ On failure:
        ├─ Log error to fetch_task_logs
        └─ If attempt_count &lt; max_attempts:
            │   └─ Set status = 'pending' (retry)
            └─ Else:
                └─ Set status = 'failed' (permanent)
```

### 4. Complete or Release Task

```
All components completed?
    ├─ YES: mark_task_completed()
    │       └─ Set task status = 'completed'
    └─ NO: release_task()
            └─ Set task status = 'pending'
                (Will be picked up again for incomplete components)
```

## 🔁 Retry Mechanisms

### Component-Level Retry

**Default Configuration**:
- `max_attempts`: 3
- Retry on: Network errors, HTTP 5xx, timeout
- No retry on: 404 (not found), 403 (forbidden), malformed data

**Retry Decision Matrix**:

| Error Type | Retry | Reason |
|------------|-------|--------|
| Network timeout | ✅ | Transient network issue |
| HTTP 503 | ✅ | Service temporarily unavailable |
| HTTP 500 | ✅ | Server error, may recover |
| HTTP 404 | ❌ | Paper doesn't exist |
| HTTP 403 | ❌ | Access denied |
| Parse error | ❌ | Data format issue |

### Timeout-Based Retry

Tasks have `last_processed_at` timestamp preventing immediate re-processing:

```rust
// Blueprint configuration
pub struct Blueprint {
    pub task_timeout_secs: u64,  // Default: 1 second
    // ...
}
```

This prevents:
- Tight retry loops overwhelming NCBI API
- Workers competing for same task
- Database lock contention

### Stale Task Recovery

Heartbeat mechanism detects crashed workers:

```sql
-- Release tasks with stale heartbeats
UPDATE fetch_tasks 
SET status = 'pending', 
    worker_id = NULL,
    heartbeat_at = NULL
WHERE status = 'in_progress' 
  AND heartbeat_at &lt; NOW() - INTERVAL '300 seconds';
```

## 🔍 PubMed Fetching Details

### NCBI E-utilities Integration

**Base URL**: `https://eutils.ncbi.nlm.nih.gov/entrez/eutils`

**API Endpoints Used**:

1. **ESummary** (`/esummary.fcgi`):
   - Search PMC database with query
   - Returns list of PMC IDs and metadata
   - Rate limit: 3 requests/second (without API key)

2. **EFetch** (`/efetch.fcgi`):
   - Fetches full abstract in XML format
   - Extracts structured abstract sections
   - Converts to markdown for readability

3. **OA Service** (`/pmc/utils/oa/oa.fcgi`):
   - Queries Open Access service for PDF URL
   - Returns FTP or HTTP link to full text

### Rate Limiting

Implemented via `tokio::time::sleep`:

```rust
// 500ms delay between requests (conservative)
tokio::time::sleep(Duration::from_millis(500)).await;
```

**Best Practices**:
- Use NCBI API key (increases limit to 10 req/s)
- Current implementation hardcodes key (should be env var)
- Consider token bucket for burst capacity

### Abstract Processing

XML parsing with tag removal:

```
Input (XML):
<abstract>
  <title>Background</title>
  Motor cortex plays a critical role...
  <title>Methods</title>
  We analyzed 50 patients...
</abstract>

Output (Markdown):
**Background**
Motor cortex plays a critical role...

**Methods**
We analyzed 50 patients...
```

### PDF Handling

**PDF URL Discovery**:
```xml
&lt;link format="pdf" updated="2023-01-15"&gt;
  ftp://ftp.ncbi.nlm.nih.gov/pub/pmc/PMC123456.pdf
&lt;/link&gt;
```

**URL Conversion**:
- Converts FTP → HTTPS for firewall compatibility
- Validates Content-Type header
- Streams directly to S3 (no local storage)

**Error Handling**:
- Detects retracted articles
- Handles OA service errors gracefully
- Falls back to alternative PDF sources

## 📡 API Endpoints

### Health Check
```
GET /fetcher-be/health
Response: { "status": "healthy" }
```

### Enqueue Query
```
POST /api/queue/enqueue
Body: {
  "query": "motor cortex AND stroke",
  "priority": 5
}
Response: {
  "task_ids": ["uuid1", "uuid2", ...],
  "pmc_ids": ["PMC123", "PMC456", ...]
}
```

### Queue Status
```
GET /api/queue/status
Response: {
  "total_tasks": 150,
  "pending": 50,
  "in_progress": 10,
  "completed": 85,
  "failed": 5,
  "avg_completion_time_secs": 45.2
}
```

### Task Details
```
GET /api/queue/task/{pmc_id}?query={query}
Response: {
  "task_id": "uuid",
  "pmc_id": "PMC123456",
  "query": "motor cortex",
  "status": "completed",
  "components": [
    {
      "type": "summary",
      "status": "completed",
      "s3_key": "papers/PMC123456/summary",
      "attempts": 1
    },
    // ... abstract, pdf
  ]
}
```

### Get Task Components
```
GET /api/queue/task/{task_id}/components
Response: {
  "task_id": "uuid",
  "components": [...]
}
```

### Allocate Workers
```
POST /api/queue/workers/allocate
Body: {
  "worker_count": 5,
  "task_timeout_secs": 300,
  "max_retry_attempts": 3
}
Response: {
  "allocated": 5,
  "worker_ids": ["uuid1", "uuid2", ...]
}
```

### Stop Workers
```
POST /api/queue/workers/stop
Body: {
  "worker_ids": ["uuid1"]  // Empty array = stop all
}
Response: {
  "stopped": 1
}
```

### Worker Status
```
GET /api/queue/workers/status
Response: [
  {
    "id": "uuid",
    "status": "busy",
    "current_task": "PMC123456",
    "tasks_completed": 42,
    "tasks_failed": 3,
    "uptime_secs": 3600
  }
]
```

## ⚙️ Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | Required |
| `S3_ENDPOINT` | S3 endpoint URL | Required |
| `S3_ACCESS_KEY` | S3 access key | Required |
| `S3_SECRET_KEY` | S3 secret key | Required |
| `S3_BUCKET` | S3 bucket name | Required |
| `FETCHER_HTTP_ADDR` | HTTP bind address | `0.0.0.0:8080` |
| `RUST_LOG` | Logging level | `info` |

### Blueprint Configuration

Runtime configuration via `Blueprint` struct:

```rust
pub struct Blueprint {
    pub task_timeout_secs: u64,        // Default: 1
    pub empty_queue_sleep_secs: u64,   // Default: 5
    pub max_retry_attempts: usize,     // Default: 3
    pub stale_task_multiplier: u64,    // Default: 10
    pub backoff_strategy: BackoffStrategy,  // Default: Constant
}
```

### S3 Key Pattern

```
{bucket}/papers/{pmc_id}/{component_type}

Examples:
cortexmap-papers/papers/PMC123456/summary
cortexmap-papers/papers/PMC123456/abstract
cortexmap-papers/papers/PMC123456/pdf
```

## 🧪 Testing

### Unit Tests
```bash
cargo test
```

### Integration Tests
```bash
# Start test infrastructure
docker-compose -f ../docker-compose.test.yml up -d postgres minio

# Run tests
cargo test --features integration

# Cleanup
docker-compose -f ../docker-compose.test.yml down -v
```

### Manual Testing

**Enqueue a task**:
```bash
curl -X POST http://localhost:8080/api/queue/enqueue \
  -H "Content-Type: application/json" \
  -d '{"query": "motor cortex AND stroke", "priority": 8}'
```

**Check queue status**:
```bash
curl http://localhost:8080/api/queue/status | jq
```

**Allocate workers**:
```bash
curl -X POST http://localhost:8080/api/queue/workers/allocate \
  -H "Content-Type: application/json" \
  -d '{"worker_count": 2, "task_timeout_secs": 60, "max_retry_attempts": 3}'
```

**Monitor workers**:
```bash
watch -n 2 'curl -s http://localhost:8080/api/queue/workers/status | jq'
```

## 🐛 Troubleshooting

### Workers Not Processing Tasks

**Symptoms**: Tasks stuck in `pending` status

**Checks**:
```bash
# Verify workers are allocated
curl http://localhost:8080/api/queue/workers/status

# Check for stale heartbeats
psql cortexmap -c "SELECT id, status, heartbeat_at FROM fetch_tasks WHERE status = 'in_progress';"
```

**Solutions**:
1. Allocate workers if none exist
2. Release stale tasks: manually update `status = 'pending'` where `heartbeat_at` is old
3. Check logs for worker errors: `grep ERROR /tmp/cortexmap-be.log`

### Components Failing Permanently

**Symptoms**: Components stuck in `failed` status

**Investigation**:
```sql
-- Find failed components
SELECT tc.task_id, tc.component_type, tc.error_message, tc.attempt_count
FROM fetch_task_components tc
WHERE tc.status = 'failed';

-- Check logs for details
SELECT * FROM fetch_task_logs 
WHERE task_id = 'failing-uuid' 
ORDER BY created_at DESC;
```

**Common Causes**:
- Paper not in Open Access (PDF unavailable)
- PMC ID invalid or retracted
- Network issues exhausted retry attempts
- S3 credentials invalid

### S3 Upload Failures

**Symptoms**: Component fetch succeeds but S3 upload fails

**Debug**:
```bash
# Test S3 connectivity
aws s3 ls s3://$S3_BUCKET/ --endpoint-url=$S3_ENDPOINT

# Check S3 credentials
echo $S3_ACCESS_KEY
echo $S3_SECRET_KEY
```

**Solutions**:
- Verify S3 credentials are correct
- Check bucket exists and is accessible
- Ensure network allows outbound S3 connections
- For MinIO: verify `force_path_style` is enabled

### High Database Connection Count

**Symptoms**: "too many connections" errors

**Investigation**:
```sql
SELECT count(*) FROM pg_stat_activity WHERE datname = 'cortexmap';
```

**Solutions**:
1. Reduce r2d2 pool size in code
2. Increase PostgreSQL `max_connections`
3. Reduce worker count
4. Optimize query patterns to release connections faster

## 📈 Performance Optimization

### Worker Scaling

**Horizontal Scaling**:
- Multiple fetcher-be instances with shared PostgreSQL
- `FOR UPDATE SKIP LOCKED` prevents duplicate work
- Workers coordinate via database, no inter-process communication needed

**Optimal Worker Count**:
```
workers = min(
  CPU cores × 2,
  NCBI rate limit / avg_task_duration,
  database connection pool size
)
```

Example:
- 4 CPU cores
- Rate limit: 3 req/s with API key → 10 req/s
- Avg task duration: 5 seconds
- Database pool: 10 connections

**Calculation**: `min(8, 10/5, 10) = min(8, 2, 10) = 2 workers optimal`

### Database Optimization

**Connection Pooling**:
```rust
// Tune based on workload
let pool = Pool::builder()
    .max_size(10)  // Increase for more workers
    .min_idle(Some(2))
    .connection_timeout(Duration::from_secs(30))
    .build(manager)?;
```

**Query Optimization**:
- Ensure `idx_fetch_tasks_queue` index exists
- Monitor slow queries: `pg_stat_statements`
- Consider partitioning for large task volumes

### S3 Upload Optimization

Currently sequential per task. Consider:
- Parallel component uploads within a task
- Multipart uploads for large PDFs
- Connection pooling for S3 client

## 🚀 Deployment

### Docker Build

```bash
# Build image
docker build -t fetcher-be:latest -f Dockerfile .

# Run container
docker run -p 8080:8080 \
  -e DATABASE_URL="..." \
  -e S3_ENDPOINT="..." \
  fetcher-be:latest
```

### Health Check

Docker Compose health check:
```yaml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:8080/fetcher-be/health"]
  interval: 30s
  timeout: 5s
  retries: 3
```

### Production Considerations

1. **NCBI API Key**: Move hardcoded key to environment variable
2. **Rate Limiting**: Implement token bucket for burst capacity
3. **Monitoring**: Export Prometheus metrics
4. **Logging**: Structured JSON logs for aggregation
5. **Backoff**: Implement exponential backoff (currently configured but not used)

## 🤝 Contributing

When contributing to fetcher-be:

1. Maintain clean separation between crates
2. Add comprehensive error handling
3. Write unit tests for new fetching logic
4. Update database migrations for schema changes
5. Document retry behavior and error conditions
6. Run `cargo fmt` and `cargo clippy` before committing

---

**Fetcher Backend** - Reliable, scalable paper acquisition for neuroscience research
```

---

## Orchestrator README (`/orch/README.md`)

```markdown
# Orchestrator (Orch)

> Central coordination service managing the CortexMap pipeline from paper fetching to summary generation

The Orchestrator is the brain of CortexMap, coordinating the entire workflow between the fetcher and brainatlas services. It implements batch processing, background monitoring, health checking, and provides a unified API for frontend clients.

## 🎯 Purpose

This service:

1. **Coordinates pipeline** between fetcher-be and brainatlas-be
2. **Generates search queries** using LLM for each brain region
3. **Manages batches** to prevent redundant processing
4. **Monitors completion** via background watchers
5. **Provides unified API** for frontend clients
6. **Handles configuration** for pipeline tuning
7. **Health checks** dependent services before starting

## 🏗️ Architecture

### Hexagonal Architecture

```
┌─────────────────────────────────────────┐
│           server/                       │  HTTP Layer
│      (Axum, Routes, Health)             │
└───────────────┬─────────────────────────┘
                │
┌───────────────▼─────────────────────────┐
│               api/                       │  API Layer
│        (OrchApi Trait)                   │
│    + Background Loop Initialization      │
└───────────────┬─────────────────────────┘
                │
┌───────────────▼─────────────────────────┐
│              app/                        │  Application
│  (Batch Management, Query Generation)    │
└───┬───────────────────────────────┬─────┘
    │                               │
┌───▼──────────────┐     ┌──────────▼──────┐
│   services/      │     │   domain/       │
│ (Fetcher Client, │     │  (Models,       │
│  BrainAtlas RPC, │     │   Batch States) │
│  Config Mgmt)    │     └─────────────────┘
└───┬──────────────┘
    │
┌───▼──────────────────────────────────────┐
│              infra/                       │
│  (DB, Redis, HTTP Clients)               │
└───────────────────────────────────────────┘
```

### Background Watchers

Two critical background loops monitor pipeline progress:

1. **Completion Watcher**: Polls fetcher for completed tasks, triggers brainatlas processing
2. **Region Scanner**: Periodically scans regions to regenerate stale summaries

```
[Completion Watcher Loop]
    Every 30s:
    ├─ Query batches in "collecting" state
    ├─ Check if all fetch tasks completed
    ├─ If complete → trigger brainatlas processing
    └─ Update batch status

[Region Scanner Loop]
    Every 24h:
    ├─ Find regions with no summary or old summary
    ├─ Generate new queries via LLM
    ├─ Create batch and enqueue to fetcher
    └─ Track progress
```

## 🚀 Getting Started

### Prerequisites

- Rust 1.75+ (2024 edition)
- PostgreSQL 15+
- Redis 7+
- Running fetcher-be and brainatlas-be services

### Installation

1. **Set up database**:
   ```bash
   # Run orch migrations
   cd orch
   diesel migration run
   ```

2. **Configure environment** (`.env.orch`):
   ```bash
   # Database
   DATABASE_URL=postgres://user:password@localhost/cortexmap
   
   # Service URLs
   FETCHER_HTTP_ADDR=http://localhost:8080
   BRAINATLAS_HTTP_ADDR=http://localhost:8081
   ORCH_HTTP_ADDR=0.0.0.0:8082
   
   # Redis Cache
   REDIS_URL=redis://localhost:6379
   
   # Logging
   RUST_LOG=info
   ```

3. **Build and run**:
   ```bash
   cargo build --release
   cargo run --bin orch
   ```

The service will:
- Check health of fetcher-be and brainatlas-be
- Initialize database connection and Redis cache
- Start background watchers
- Begin serving HTTP API on port 8082

## 📊 Database Schema

### Tables

#### `region_processing_batches`
Tracks batches of fetch tasks for a brain region.

```sql
CREATE TABLE region_processing_batches (
  id UUID PRIMARY KEY,
  region_id INTEGER NOT NULL,
  status VARCHAR(50) DEFAULT 'collecting',  -- State machine
  fetch_task_ids UUID[] DEFAULT '{}',
  expected_task_count INTEGER,
  error_message TEXT,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  completed_at TIMESTAMP,
  
  -- Ensure one active batch per region
  CONSTRAINT unique_active_batch 
    UNIQUE (region_id) 
    WHERE (status IN ('collecting', 'ready', 'processing'))
);

CREATE INDEX idx_batches_region ON region_processing_batches(region_id);
CREATE INDEX idx_batches_status ON region_processing_batches(status);
```

**Batch State Machine**:
```
collecting → ready → processing → completed
                                → failed
```

- **collecting**: Accumulating fetch tasks
- **ready**: All tasks completed, ready for brainatlas
- **processing**: Brainatlas is generating summary
- **completed**: Summary generated successfully
- **failed**: Error during processing

#### `region_queries`
LLM-generated search queries for each region.

```sql
CREATE TABLE region_queries (
  id UUID PRIMARY KEY,
  region_id INTEGER NOT NULL,
  query_text TEXT NOT NULL,
  source VARCHAR(50) DEFAULT 'llm_generated',  -- or 'user_added', 'user_modified'
  enabled BOOLEAN DEFAULT true,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  UNIQUE (region_id, query_text)
);

CREATE INDEX idx_queries_region ON region_queries(region_id);
CREATE INDEX idx_queries_enabled ON region_queries(region_id, enabled);
```

#### `processed_fetch_tasks`
Tracks which fetch tasks have been processed to avoid duplicates.

```sql
CREATE TABLE processed_fetch_tasks (
  fetch_task_id UUID PRIMARY KEY,
  batch_id UUID NOT NULL REFERENCES region_processing_batches(id),
  processed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_processed_batch ON processed_fetch_tasks(batch_id);
```

#### `orch_config`
Runtime configuration key-value store.

```sql
CREATE TABLE orch_config (
  key VARCHAR(100) PRIMARY KEY,
  value TEXT NOT NULL,
  description TEXT,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Default configuration
INSERT INTO orch_config (key, value, description) VALUES
  ('completion_check_interval_secs', '30', 'How often to check for completed batches'),
  ('region_scan_interval_hours', '24', 'How often to scan regions for updates'),
  ('summary_stale_after_days', '30', 'Age threshold for re-generating summaries'),
  ('queries_per_region', '3', 'Number of queries to generate per region');
```

## 🔄 Pipeline Workflow

### Complete End-to-End Flow

```
1. Frontend: User clicks "Generate Summary" for a region
   │
   ▼
2. Orch: POST /regions/{id}/generate
   ├─ Generate queries via brainatlas LLM (count: 3)
   ├─ Create batch (status: "collecting")
   ├─ For each query:
   │   └─ Enqueue to fetcher-be
   └─ Return batch_id to frontend
   │
   ▼
3. Fetcher: Workers process tasks
   ├─ Fetch PDFs, abstracts, summaries from PubMed
   └─ Upload to S3
   │
   ▼
4. Orch: Completion Watcher detects finished batch
   ├─ Query fetcher for task statuses
   ├─ If all tasks completed:
   │   ├─ Update batch status = "ready"
   │   └─ Trigger brainatlas processing
   │
   ▼
5. BrainAtlas: Process region
   ├─ Download S3 files
   ├─ Generate embeddings
   ├─ Run RAG loop to create summary
   └─ Return summary_id
   │
   ▼
6. Orch: Update batch status = "completed"
   │
   ▼
7. Frontend: Poll batch status, display summary when ready
```

### Query Generation

Orch delegates query generation to brainatlas LLM service:

```
Orch → POST /brainatlas-be/api/generate-queries
Body: {
  "region_name": "Motor Cortex",
  "region_acronym": "MC",
  "count": 3
}

BrainAtlas → Uses LLM with tool calling
  ├─ Tool: create_pubmed_query
  ├─ Generates structured queries
  └─ Returns PubMed-formatted strings

Response: {
  "queries": [
    "motor+cortex+AND+(function+OR+anatomy)",
    "motor+cortex+AND+(stroke+OR+rehabilitation)",
    "motor+cortex+AND+(neuroimaging+OR+fMRI)"
  ]
}

Orch → Stores queries in region_queries table
```

### Batch Processing

**Batch Creation**:
```rust
// Create batch
let batch_id = create_batch(region_id, expected_task_count);

// Enqueue queries to fetcher
for query in queries {
    let task_ids = fetcher.enqueue_query(query, priority);
    add_tasks_to_batch(batch_id, task_ids);
}

// Background watcher monitors completion
```

**Completion Detection**:
```rust
// Completion watcher loop (every 30s)
for batch in get_collecting_batches() {
    let statuses = fetcher.get_task_statuses(batch.task_ids);
    
    if all_completed(statuses) {
        // Trigger brainatlas processing
        let summary_id = brainatlas.process_region(
            region_id,
            s3_keys_from_tasks
        );
        
        update_batch_status(batch_id, "completed");
    }
}
```

## 📡 API Endpoints

### Health Check
```
GET /orch/health
Response: { "status": "healthy", "dependencies": {...} }
```

### List Brain Regions
```
GET /orch/api/regions
Response: [
  {
    "id": "uuid",
    "region_id": 123,
    "name": "Motor Cortex",
    "acronym": "MC",
    "color": {"red": 255, "green": 100, "blue": 50},
    ...
  }
]
```

### Get Region Status
```
GET /orch/api/regions/{id}/status
Response: {
  "region_id": "uuid",
  "status": "Done",  // Pipeline status
  "summary_count": 5,
  "last_fetch_at": "2026-02-20T10:30:00Z",
  "last_summary_at": "2026-02-20T12:45:00Z"
}
```

**Pipeline Status Values**:
- `NotStarted`: No processing initiated
- `FetchQueued`: Queries enqueued to fetcher
- `Fetching`: Workers downloading papers
- `FetchFailed`: Fetch errors occurred
- `LlmQueued`: Waiting for brainatlas processing
- `Processing`: Brainatlas generating summary
- `Done`: Summary available
- `Invalidated`: Marked for reprocessing

### Get Region Summaries
```
GET /orch/api/regions/{id}/summaries
Response: {
  "summaries": [
    {
      "id": "uuid",
      "region_id": 123,
      "summary_text": "# Motor Cortex\n\n...",
      "created_at": "2026-02-20T12:45:00Z",
      "batch_id": "uuid"
    }
  ]
}
```

### Generate Region Summary
```
POST /orch/api/regions/{id}/generate
Body: {} (optional priority)
Response: {
  "batch_id": "uuid",
  "status": "collecting",
  "message": "Summary generation started"
}
```

**Workflow**:
1. Generate queries via LLM (3 queries)
2. Create batch record
3. Enqueue each query to fetcher with priority
4. Return batch_id for polling

### Invalidate Region
```
POST /orch/api/regions/{id}/invalidate
Body: {}
Response: {
  "batch_id": "uuid",
  "message": "Region invalidated, new summary generation started"
}
```

Forces regeneration even if summary exists. Useful for:
- Updated research available
- Previous summary quality issues
- Schema/prompt changes

### Get Batch Status
```
GET /orch/api/batches/{batch_id}/status
Response: {
  "batch_id": "uuid",
  "status": "processing",
  "progress": 75,
  "message": "Generating summary from 15 papers",
  "created_at": "...",
  "expected_tasks": 15,
  "completed_tasks": 15
}
```

### Pipeline Statistics
```
GET /orch/api/pipeline/stats
Response: {
  "total_regions": 1034,
  "done": 523,
  "processing": 12,
  "llm_queued": 5,
  "fetching": 8,
  "fetch_queued": 20,
  "not_started": 450,
  "fetch_failed": 3,
  "invalidated": 13
}
```

### Configuration Management

**Get Configuration**:
```
GET /orch/api/config
Response: {
  "completion_check_interval_secs": "30",
  "region_scan_interval_hours": "24",
  "summary_stale_after_days": "30",
  "queries_per_region": "3"
}
```

**Update Configuration**:
```
PUT /orch/api/config
Body: {
  "completion_check_interval_secs": "60"
}
Response: { "updated": true }
```

### Worker Management (Proxied to Fetcher)

```
POST /orch/api/workers/allocate
GET /orch/api/workers/status
POST /orch/api/workers/stop
```

Orch proxies these to fetcher-be for unified API.

## ⚙️ Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | Required |
| `FETCHER_HTTP_ADDR` | Fetcher service URL | Required |
| `BRAINATLAS_HTTP_ADDR` | BrainAtlas service URL | Required |
| `ORCH_HTTP_ADDR` | HTTP bind address | `0.0.0.0:8082` |
| `REDIS_URL` | Redis connection string | `redis://localhost:6379` |
| `RUST_LOG` | Logging level | `info` |

### Runtime Configuration (Database)

Tunable via API or directly in database:

```sql
UPDATE orch_config 
SET value = '60' 
WHERE key = 'completion_check_interval_secs';
```

**Key Configuration Parameters**:

- **completion_check_interval_secs** (default: 30)
  - How often completion watcher checks batches
  - Lower = faster response, higher DB load
  
- **region_scan_interval_hours** (default: 24)
  - Frequency of automatic region scanning
  - Determines how often stale summaries are refreshed
  
- **summary_stale_after_days** (default: 30)
  - Age threshold for considering summaries outdated
  - Older summaries trigger regeneration
  
- **queries_per_region** (default: 3)
  - Number of PubMed queries generated per region
  - More queries = broader paper coverage, longer processing

### Redis Caching

**Cache Strategy**: Cache-aside pattern

**Cached Data**:
- Region metadata (list of regions)
- Pipeline statistics
- Configuration values

**TTL Configuration**:
```rust
// Example cache usage
redis.set_ex("regions:list", json, 300).await?;  // 5 min TTL
```

**Cache Invalidation**:
- Automatic on TTL expiry
- Manual via background watchers on updates

## 🧪 Testing

### Unit Tests
```bash
cargo test
```

### Integration Tests
```bash
# Start all dependencies
docker-compose -f ../docker-compose.test.yml up -d

# Run orch tests
cargo test --features integration

# Cleanup
docker-compose -f ../docker-compose.test.yml down -v
```

### Manual End-to-End Test

1. **Start all services**:
   ```bash
   # Terminal 1: Fetcher
   cd fetcher-be && cargo run
   
   # Terminal 2: BrainAtlas
   cd brainatlas-be && cargo run
   
   # Terminal 3: Orch
   cd orch && cargo run
   ```

2. **Allocate workers**:
   ```bash
   curl -X POST http://localhost:8082/orch/api/workers/allocate \
     -H "Content-Type: application/json" \
     -d '{"worker_count": 2, "task_timeout_secs": 60, "max_retry_attempts": 3}'
   ```

3. **Trigger summary generation**:
   ```bash
   # Get a region ID first
   REGION_ID=$(curl -s http://localhost:8082/orch/api/regions | jq -r '.[0].id')
   
   # Generate summary
   curl -X POST http://localhost:8082/orch/api/regions/$REGION_ID/generate
   ```

4. **Monitor progress**:
   ```bash
   # Watch pipeline stats
   watch -n 2 'curl -s http://localhost:8082/orch/api/pipeline/stats | jq'
   
   # Check region status
   watch -n 2 "curl -s http://localhost:8082/orch/api/regions/$REGION_ID/status | jq"
   ```

5. **View summary**:
   ```bash
   curl http://localhost:8082/orch/api/regions/$REGION_ID/summaries | jq
   ```

## 🐛 Troubleshooting

### Orch Won't Start

**Symptoms**: Service exits immediately after health checks

**Checks**:
```bash
# Check if dependencies are healthy
curl http://localhost:8080/fetcher-be/health
curl http://localhost:8081/brainatlas-be/health

# View orch logs
tail -f /tmp/orch.log
```

**Common Causes**:
- Fetcher or BrainAtlas not running
- Wrong service URLs in environment
- Database migration not run
- Redis not accessible

### Batches Stuck in "Collecting"

**Symptoms**: Batches never transition to "ready"

**Investigation**:
```sql
-- Check batch status
SELECT id, region_id, status, expected_task_count, 
       array_length(fetch_task_ids, 1) as actual_count
FROM region_processing_batches 
WHERE status = 'collecting';

-- Check if fetch tasks are completed
-- (Query fetcher database or use API)
```

**Common Causes**:
- Fetch tasks failed (check fetcher logs)
- Workers not allocated
- Completion watcher not running
- Task IDs in batch don't match actual task IDs

### Background Watchers Not Running

**Symptoms**: No automatic processing, must manually trigger

**Debug**:
```bash
# Check if api.init() was called
grep "Starting background" /tmp/orch.log

# Verify intervals are reasonable
curl http://localhost:8082/orch/api/config | jq
```

**Solutions**:
- Ensure `api.init().await` is called in main.rs
- Check configuration intervals aren't too long
- Verify no panics in watcher loops (check logs)

### Redis Connection Issues

**Symptoms**: Errors mentioning Redis, cache misses

**Checks**:
```bash
# Test Redis connectivity
redis-cli -u redis://localhost:6379 ping

# Check environment
echo $REDIS_URL
```

**Solutions**:
- Verify Redis is running: `docker ps | grep redis`
- Check REDIS_URL format: `redis://host:port`
- Ensure network connectivity
- For Docker: use service name, not localhost

## 📈 Performance Optimization

### Completion Watcher Tuning

**Default**: 30 seconds

**Considerations**:
- **Lower interval (10-15s)**: Faster response, higher DB load, more API calls
- **Higher interval (60s+)**: Reduced load, slower summary generation

**Recommendation**: 
- Development: 10-15s for quick testing
- Production: 30-60s for balanced performance

### Redis Cache Strategy

**Current Implementation**: Simple key-value with TTL

**Optimization Ideas**:
1. **Cache warming**: Pre-populate frequently accessed data
2. **Hierarchical caching**: Different TTLs for different data types
3. **Cache stampede prevention**: Lock-based updates

### Database Connection Pooling

```rust
// Tune based on workload
let pool = deadpool_diesel::postgres::Pool::builder(manager)
    .max_size(20)  // Increase for high traffic
    .build()?;
```

### Batch Size Limits

Consider adding:
```sql
-- Limit number of tasks per batch
ALTER TABLE region_processing_batches 
  ADD CONSTRAINT max_tasks_per_batch 
  CHECK (expected_task_count <= 100);
```

Prevents:
- Single batch overwhelming system
- LLM processing timeouts
- Memory issues

## 🚀 Deployment

### Docker Build

```bash
# Build image
docker build -t orch:latest -f Dockerfile .

# Run container
docker run -p 8082:8082 \
  -e DATABASE_URL="..." \
  -e REDIS_URL="redis://redis:6379" \
  -e FETCHER_HTTP_ADDR="http://fetcher:8080" \
  -e BRAINATLAS_HTTP_ADDR="http://brainatlas:8081" \
  orch:latest
```

### Health Checks

**Startup Health Checks**:
```rust
// In main.rs
if let Err(e) = api.fetcher_health().await {
    tracing::error!("Fetcher unhealthy: {}", e);
    std::process::exit(1);
}
```

**Runtime Health Checks**:
```yaml
# docker-compose.yml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:8082/orch/health"]
  interval: 30s
  timeout: 5s
  retries: 3
```

### Graceful Shutdown

Background watchers should handle signals:

```rust
// TODO: Implement graceful shutdown
// - Cancel background tasks
// - Drain in-flight requests
// - Close database connections
```

## 🤝 Contributing

When contributing to orch:

1. Follow hexagonal architecture principles
2. Test background watchers thoroughly
3. Document state machine transitions
4. Add database migrations for schema changes
5. Update API documentation
6. Run `cargo fmt` and `cargo clippy`

### Adding New Background Watchers

Pattern to follow:

```rust
// In api/src/api.rs
pub async fn init(&self) -> Result<(), Error> {
    let services = self.services.clone();
    
    tokio::spawn(async move {
        loop {
            // Watcher logic
            if let Err(e) = services.some_background_task().await {
                tracing::error!("Background task failed: {}", e);
            }
            
            // Sleep interval
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    });
    
    Ok(())
}
```

---

**Orchestrator** - Coordinating the CortexMap pipeline for seamless neuroscience research
```

---

This completes all README documentation for the CortexMap project.

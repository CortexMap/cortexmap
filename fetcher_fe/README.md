# CortexMap Fetcher Frontend

Frontend for monitoring paper fetching progress and searching brain regions.

## Quick Start

```bash
npm install
npm start
```

Access at http://localhost:3000/fetcher-fe

## Backend

- **URL**: https://capstone.ssdd.dev/fetcher-be
- **Base Path**: `/fetcher-fe`
- **Protocol**: REST over gRPC

## Features

### Search & Query
- PubMed query search
- Configurable paper count (1-20)
- Query history with deduplication
- Real-time status updates (200ms polling)

### Workers Management
- Add/remove workers dynamically
- Live worker status (2s polling)
- Worker productivity metrics

### Paper Cards
- Markdown-rendered summaries
- Collapsible abstract sections
- Status tracking per paper

### Queue Statistics
- Total, pending, in-progress, completed, failed tasks
- Component-level stats (summary, abstract, PDF)
- Worker statistics

## Routes

- `/` - Home page with search and workers
- `/query` - Query results with paper cards
- `/history` - Browse past queries

## API Endpoints

- `POST /api/queue/enqueue` - Submit query
- `GET /api/queue/status` - Queue stats with recent tasks
- `GET /api/queue/task/{pmc_id}` - Task details
- `POST /api/queue/workers/allocate` - Start workers
- `POST /api/queue/workers/stop` - Stop workers
- `GET /api/queue/workers/status` - Worker info

## Tech Stack

- React 18 + TypeScript
- Vite
- React Router
- React Markdown
- LocalStorage for history

## Project Structure

```
src/
├── api/
│   └── backendApi.ts          # API client
├── components/
│   ├── SearchBar.tsx          # Query input with page size
│   ├── PaperCard.tsx          # Paper display with markdown
│   ├── PaperList.tsx          # Paper grid
│   └── WorkersSection.tsx     # Workers & queue stats
├── hooks/
│   └── usePaperFetcher.ts     # Main logic hook
├── pages/
│   ├── HomePage.tsx           # Search page
│   ├── QueryPage.tsx          # Results page
│   └── HistoryPage.tsx        # History browser
├── services/
│   └── queryHistory.ts        # LocalStorage service
├── types/
│   ├── index.ts               # Core types
│   └── enhanced.ts            # Extended types
└── main.tsx                   # Entry point
```

## Configuration

- **Poll Interval**: 200ms (papers)
- **Worker Poll**: 2s (workers/queue)
- **Max Retries**: 0
- **Default Papers**: 3
- **Max Papers**: 20
- **History Limit**: 50 queries

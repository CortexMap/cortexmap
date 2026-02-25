# Fetcher Backend

> High-performance PubMed paper fetching service with distributed worker pool architecture

The Fetcher Backend is a Rust-based service that manages a sophisticated task queue for downloading academic papers from PubMed Central (PMC). It features a scalable worker pool, component-based processing with intelligent retry mechanisms, and S3 storage integration.

For comprehensive documentation and the main project overview, see the [main README](../README.md).

## Quick Links

- [Main Project README](../README.md) - Project overview and architecture
- [BrainAtlas Backend](../brainatlas-be/README.md) - LLM processing service
- [Orchestrator](../orch/README.md) - Pipeline coordination
- [Frontend](../brainatlas-fe/README.md) - User interface

## Purpose

This service:

1. **Accepts queries** for neuroscience papers from PubMed
2. **Enqueues tasks** with priority-based scheduling
3. **Manages workers** that process tasks concurrently
4. **Fetches components**: PDF, abstract, and AI-generated summary
5. **Stores results** in S3-compatible storage
6. **Handles failures** with sophisticated retry logic

## Architecture

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
    │ (Tokio Task)│       │ (Tokio Task)│
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

## Getting Started

See the [main README](../README.md#quick-start) for setup instructions.

### Quick Setup

1. **Install Diesel CLI**: `cargo install diesel_cli --no-default-features --features postgres`
2. **Run migrations**: `diesel migration run`
3. **Configure environment** (see Configuration section below)
4. **Build**: `cargo build --release`
5. **Run**: `cargo run --bin cortexmap-be`
6. **Allocate workers**:
   ```bash
   curl -X POST http://localhost:8080/api/queue/workers/allocate \
     -H "Content-Type: application/json" \
     -d '{"worker_count": 2, "task_timeout_secs": 300, "max_retry_attempts": 3}'
   ```

## Configuration

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

## Key Features

### Component-Based Processing

Each task fetches three independent components:
- **Summary**: PubMed abstract
- **Abstract**: Extracted from XML
- **PDF**: Full paper from Open Access

Each component has independent retry logic, allowing partial success.

### Worker Pool Management

- Dynamic worker allocation via HTTP API
- Graceful shutdown using cancellation tokens
- Worker statistics with database query integration
- Each worker runs in separate Tokio task

### Intelligent Retry Mechanisms

**Component-Level Retry**:
- Default: 3 attempts per component
- Timeout-based delays prevent API flooding
- Heartbeat mechanism detects crashed workers

**Retry Decision Matrix**:
- Network timeout: Retry
- HTTP 503: Retry
- HTTP 500: Retry
- HTTP 404: No retry (paper doesn't exist)
- HTTP 403: No retry (access denied)

## API Endpoints

### Queue Management
- `POST /api/queue/enqueue` - Enqueue PubMed query
- `GET /api/queue/status` - Queue statistics
- `GET /api/queue/task/{pmc_id}` - Task details
- `GET /api/queue/task/{task_id}/components` - Component status

### Worker Management
- `POST /api/queue/workers/allocate` - Spawn workers
- `POST /api/queue/workers/stop` - Stop workers
- `GET /api/queue/workers/status` - Worker statistics

### Health
- `GET /fetcher-be/health` - Health check

## Testing

```bash
# Unit tests
cargo test

# Integration tests (requires test infrastructure)
docker-compose -f ../docker-compose.test.yml up -d postgres minio
cargo test --features integration
docker-compose -f ../docker-compose.test.yml down -v
```

## Troubleshooting

### Workers Not Processing

**Check**:
```bash
curl http://localhost:8080/api/queue/workers/status
```

**Solutions**:
1. Allocate workers if none exist
2. Check logs: `grep ERROR /tmp/cortexmap-be.log`
3. Verify database connectivity

### Components Failing

**Investigate**:
```sql
-- Find failed components
SELECT tc.task_id, tc.component_type, tc.error_message, tc.attempt_count
FROM fetch_task_components tc
WHERE tc.status = 'failed';
```

**Common Causes**:
- Paper not in Open Access (PDF unavailable)
- PMC ID invalid or retracted
- S3 credentials invalid

## Performance Optimization

### Worker Scaling

Optimal worker count:
```
workers = min(
  CPU cores × 2,
  NCBI rate limit / avg_task_duration,
  database connection pool size
)
```

### Database Tuning

- Ensure `idx_fetch_tasks_queue` index exists
- Monitor slow queries with `pg_stat_statements`
- Consider partitioning for large task volumes

## Deployment

See [main README](../README.md#production-deployment) for production deployment instructions.

### Docker Build

```bash
docker build -t fetcher-be:latest -f Dockerfile .

docker run -p 8080:8080 \
  -e DATABASE_URL="..." \
  -e S3_ENDPOINT="..." \
  fetcher-be:latest
```

---

**Fetcher Backend** - Reliable, scalable paper acquisition for neuroscience research

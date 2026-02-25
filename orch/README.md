# Orchestrator (Orch)

> Central coordination service managing the CortexMap pipeline from paper fetching to summary generation

The Orchestrator is the brain of CortexMap, coordinating the entire workflow between the fetcher and brainatlas services. It implements batch processing, background monitoring, health checking, and provides a unified API for frontend clients.

For comprehensive documentation and the main project overview, see the [main README](../README.md).

## Quick Links

- [Main Project README](../README.md) - Project overview and architecture
- [BrainAtlas Backend](../brainatlas-be/README.md) - LLM processing service
- [Fetcher Backend](../fetcher-be/README.md) - Paper fetching service
- [Frontend](../brainatlas-fe/README.md) - User interface

## Purpose

This service:

1. **Coordinates pipeline** between fetcher-be and brainatlas-be
2. **Generates search queries** using LLM for each brain region
3. **Manages batches** to prevent redundant processing
4. **Monitors completion** via background watchers
5. **Provides unified API** for frontend clients
6. **Handles configuration** for pipeline tuning
7. **Health checks** dependent services before starting

## Architecture

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

## Getting Started

See the [main README](../README.md#quick-start) for setup instructions.

### Quick Setup

1. **Install Diesel CLI**: `cargo install diesel_cli --no-default-features --features postgres`
2. **Run migrations**: `diesel migration run`
3. **Configure environment** (`.env.orch`):
   ```bash
   DATABASE_URL=postgres://user:password@localhost/cortexmap
   FETCHER_HTTP_ADDR=http://localhost:8080
   BRAINATLAS_HTTP_ADDR=http://localhost:8081
   ORCH_HTTP_ADDR=0.0.0.0:8082
   REDIS_URL=redis://localhost:6379
   RUST_LOG=info
   ```
4. **Build**: `cargo build --release`
5. **Run**: `cargo run --bin orch`

The service will:
- Check health of fetcher-be and brainatlas-be
- Initialize database connection and Redis cache
- Start background watchers
- Begin serving HTTP API on port 8082

## Pipeline Workflow

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

## Database Schema

### Key Tables

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
  
  -- Ensure one active batch per region
  CONSTRAINT unique_active_batch 
    UNIQUE (region_id) 
    WHERE (status IN ('collecting', 'ready', 'processing'))
);
```

**Batch State Machine**:
```
collecting → ready → processing → completed
                                → failed
```

#### `region_queries`
LLM-generated search queries for each region.

```sql
CREATE TABLE region_queries (
  id UUID PRIMARY KEY,
  region_id INTEGER NOT NULL,
  query_text TEXT NOT NULL,
  source VARCHAR(50) DEFAULT 'llm_generated',
  enabled BOOLEAN DEFAULT true,
  
  UNIQUE (region_id, query_text)
);
```

#### `orch_config`
Runtime configuration key-value store.

```sql
CREATE TABLE orch_config (
  key VARCHAR(100) PRIMARY KEY,
  value TEXT NOT NULL,
  description TEXT
);

-- Default configuration
INSERT INTO orch_config (key, value, description) VALUES
  ('completion_check_interval_secs', '30', 'How often to check for completed batches'),
  ('region_scan_interval_hours', '24', 'How often to scan regions for updates'),
  ('summary_stale_after_days', '30', 'Age threshold for re-generating summaries'),
  ('queries_per_region', '3', 'Number of queries to generate per region');
```

## API Endpoints

### Region Management
- `GET /orch/api/regions` - List all brain regions
- `GET /orch/api/regions/{id}/status` - Get region processing status
- `GET /orch/api/regions/{id}/summaries` - Get region summaries
- `POST /orch/api/regions/{id}/generate` - Start summary generation
- `POST /orch/api/regions/{id}/invalidate` - Force regeneration

### Batch Tracking
- `GET /orch/api/batches/{batch_id}/status` - Get batch status

### Pipeline Statistics
- `GET /orch/api/pipeline/stats` - Pipeline statistics

### Configuration
- `GET /orch/api/config` - Get configuration
- `PUT /orch/api/config` - Update configuration

### Worker Management (Proxied to Fetcher)
- `POST /orch/api/workers/allocate` - Allocate workers
- `GET /orch/api/workers/status` - Worker status
- `POST /orch/api/workers/stop` - Stop workers

### Health
- `GET /orch/health` - Health check

## Configuration

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

## Testing

```bash
# Unit tests
cargo test

# Integration tests (requires all services)
docker-compose -f ../docker-compose.test.yml up -d
cargo test --features integration
docker-compose -f ../docker-compose.test.yml down -v
```

## Troubleshooting

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
```

**Common Causes**:
- Fetch tasks failed (check fetcher logs)
- Workers not allocated
- Completion watcher not running

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

## Performance Optimization

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

**Cached Data**:
- Region metadata (list of regions)
- Pipeline statistics
- Configuration values

## Deployment

See [main README](../README.md#production-deployment) for production deployment instructions.

### Docker Build

```bash
docker build -t orch:latest -f Dockerfile .

docker run -p 8082:8082 \
  -e DATABASE_URL="..." \
  -e REDIS_URL="redis://redis:6379" \
  -e FETCHER_HTTP_ADDR="http://fetcher:8080" \
  -e BRAINATLAS_HTTP_ADDR="http://brainatlas:8081" \
  orch:latest
```

---

**Orchestrator** - Coordinating the CortexMap pipeline for seamless neuroscience research

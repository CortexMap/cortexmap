# CortexMap

> An AI-powered neuroscience research platform for automated brain atlas exploration and analysis

CortexMap is a distributed microservices platform that fetches, processes, and synthesizes neuroscience research papers from PubMed Central. It leverages advanced LLM capabilities (RAG, embeddings, tool calling) and vector databases to generate comprehensive, citation-backed summaries of brain regions.

## Overview

CortexMap automates the research workflow for neuroscientists by:

1. **Fetching** relevant academic papers from PubMed based on brain region queries
2. **Processing** papers through LLM-powered summarization with semantic search
3. **Generating** structured summaries with proper citations and source attribution
4. **Presenting** findings through an intuitive web interface

## Architecture

```
┌─────────────────┐
│  brainatlas-fe  │  React Frontend (Port 80)
└────────┬────────┘
         │
┌────────▼────────┐
│      orch       │  Orchestrator (Port 8082)
└────┬────┬───┬───┘
     │    │   │
     │    │   └──────────┐
     │    │              │
┌────▼────▼───┐    ┌────▼────────┐
│ brainatlas-be│    │ fetcher-be  │
│  (Port 8081) │    │ (Port 8080) │
└──────┬───────┘    └─────┬───────┘
       │                  │
┌──────▼──────────────────▼───────┐
│    PostgreSQL + pgvector + S3   │
└──────────────────────────────────┘
```

### Services

| Service | Purpose | Technology |
|---------|---------|------------|
| **[brainatlas-fe](brainatlas-fe/)** | User interface for exploring brain regions | React 19, Vite |
| **[orch](orch/)** | Pipeline orchestration and coordination | Rust, Axum, Redis |
| **[brainatlas-be](brainatlas-be/)** | LLM processing, embeddings, semantic search | Rust, OpenRouter, pgvector |
| **[fetcher-be](fetcher-be/)** | PubMed paper fetching with worker pool | Rust, NCBI E-utilities, S3 |

## Quick Start

### Prerequisites

- Docker & Docker Compose
- PostgreSQL 15+ with pgvector extension
- Redis 7+
- S3-compatible storage (AWS S3 or MinIO)
- OpenRouter API key

### Production Deployment

1. **Create external network**:
   ```bash
   docker network create infra-net
   ```

2. **Configure environment**:
   Create `.env` and `.env.orch` files (see Configuration section)

3. **Deploy services**:
   ```bash
   docker-compose -f docker-compose.app.yml up -d
   ```

4. **Run migrations**:
   ```bash
   # Fetcher migrations
   docker exec cortexmap-be diesel migration run
   
   # BrainAtlas migrations
   docker exec brainatlas-be diesel migration run
   
   # Orch migrations
   docker exec orch diesel migration run
   ```

5. **Access the application**:
   - Frontend: http://localhost:80
   - API: http://localhost:8082/orch/api

### Development Setup

1. **Start infrastructure**:
   ```bash
   docker-compose -f docker-compose.test.yml up -d postgres redis minio
   ```

2. **Run migrations**:
   ```bash
   cd fetcher-be && diesel migration run
   cd ../brainatlas-be && diesel migration run
   cd ../orch && diesel migration run
   ```

3. **Start services** (from project root):
   ```bash
   ./start-services.sh
   ```

4. **Start frontend** (separate terminal):
   ```bash
   cd brainatlas-fe && npm install && npm run dev
   ```

## Technology Stack

### Backend
- **Language**: Rust (2024 edition)
- **Web Framework**: Axum (async HTTP)
- **Database**: PostgreSQL 15 + pgvector
- **ORM**: Diesel 2.3 with r2d2 pooling
- **Vector Search**: pgvector (IVFFlat indices)
- **Storage**: AWS S3 / MinIO
- **Cache**: Redis 7
- **LLM Provider**: OpenRouter
- **Serialization**: Prost (protobuf), Serde (JSON)

### Frontend
- **Framework**: React 19.2
- **Build Tool**: Vite 7.3
- **HTTP Client**: Axios
- **Visualization**: Recharts
- **Icons**: Lucide React

### Infrastructure
- **Containerization**: Docker & Docker Compose
- **Container Registry**: AWS ECR (us-east-1)
- **Migrations**: Diesel CLI

## Repository Structure

```
cortexmap/
├── brainatlas-be/       # LLM processing & vector search service
│   ├── crates/          # Multi-crate architecture
│   │   ├── server/      # HTTP server
│   │   ├── api/         # API trait definitions
│   │   ├── app/         # Application orchestration
│   │   ├── services/    # Business logic
│   │   ├── infra/       # Infrastructure implementations
│   │   └── domain/      # Core domain models
│   ├── migrations/      # Database migrations
│   └── Dockerfile
├── brainatlas-fe/       # React frontend application
│   ├── src/
│   │   ├── components/  # React components
│   │   ├── config.js    # API configuration
│   │   └── App.jsx      # Main application
│   ├── package.json
│   └── Dockerfile
├── fetcher-be/          # PubMed paper fetching service
│   ├── crates/
│   │   ├── cortexmap-be/        # Main server
│   │   ├── cortexmap-infra/     # Infrastructure traits
│   │   ├── std-infra/           # Concrete implementations
│   │   ├── cortexmap-fetcher/   # Fetching logic
│   │   ├── cortexmap-database/  # Database models
│   │   └── cortexmap-cli/       # CLI tools
│   └── migrations/
├── orch/                # Orchestration service
│   ├── crates/          # Hexagonal architecture
│   │   ├── server/      # HTTP server
│   │   ├── api/         # API layer
│   │   ├── app/         # Application logic
│   │   ├── services/    # Service layer
│   │   ├── infra/       # Infrastructure
│   │   └── domain/      # Domain models
│   └── migrations/
├── proto/               # Protocol buffer definitions
│   ├── llm/             # LLM service contracts
│   ├── orch/            # Orchestrator contracts
│   └── app/             # Application contracts
├── tests/               # Integration tests
├── docker-compose.app.yml    # Production compose
├── docker-compose.test.yml   # Test infrastructure
├── start-services.sh         # Local development script
└── test.sh                   # Test runner
```

## Configuration

### Environment Variables

#### `.env` (Shared Configuration)
```env
# Database
DATABASE_URL=postgres://user:password@localhost/cortexmap

# S3 Storage
S3_ENDPOINT=https://s3.amazonaws.com
S3_ACCESS_KEY=your_access_key
S3_SECRET_KEY=your_secret_key
S3_BUCKET=cortexmap-papers

# LLM Provider
OPENROUTER_API_KEY=your_openrouter_key

# CORS (optional)
CORS_ORIGIN=*

# Service Addresses
FETCHER_HTTP_ADDR=0.0.0.0:8080
BRAINATLAS_HTTP_ADDR=0.0.0.0:8081
```

#### `.env.orch` (Orchestrator Configuration)
```env
# Database (orch-specific)
DATABASE_URL=postgres://user:password@localhost/cortexmap

# Service URLs (Docker network addresses for production)
FETCHER_HTTP_ADDR=http://cortexmap-be:8080
BRAINATLAS_HTTP_ADDR=http://brainatlas-be:8081
ORCH_HTTP_ADDR=0.0.0.0:8082

# Redis Cache
REDIS_URL=redis://localhost:6379

# Logging
RUST_LOG=info
```

## Data Flow

1. **User initiates summary generation** via brainatlas-fe
2. **Orch** generates search queries using LLM
3. **Orch** enqueues queries to **fetcher-be**
4. **Fetcher workers** download papers from PubMed to S3
5. **Orch** detects completion, triggers **brainatlas-be**
6. **BrainAtlas** chunks papers, generates embeddings
7. **BrainAtlas** runs RAG loop to synthesize summary
8. **User** views summary with citations in frontend

## Evaluation Metrics

Every `region_summary` row is scored by **evals-be** against a versioned suite of metrics. The set is bumped via `EVAL_VERSION` (currently `v0.3.0`); bumping the version invalidates the cache and forces re-scoring.

### Structural (no LLM — deterministic)
- `section_completeness` — fraction of required markdown sections present (Overview, Anatomy & Connectivity, Function, Clinical Relevance).
- `length_in_range` — word count inside a sane window (not too short, not bloated).
- `acronym_mention` — the region's acronym appears at least once in the body.
- `no_placeholder_text` — 0 if any LLM-failure strings ("I cannot…", "[TODO]", etc.) are present, else 1.

### Groundedness (LLM judges)
- `claim_groundedness` — atomic claims are extracted, each is re-embedded, top-k source chunks retrieved, and a judge rates `supported` / `partial` / `unsupported`. Score = supported / total.
- `hallucination_rate` — inverse: unsupported / total. Low = good.

### Rubric (LLM judge, 1–5 scale)
- `rubric_relevance` — summary stays on the named region, doesn't drift.
- `rubric_coherence` — prose is well-organised, internally consistent.
- `rubric_specificity` — concrete neuroanatomical detail vs generic filler.
- `rubric_clinical_utility` — actionable for clinicians/neuroscientists.
- `rubric_terminology` — correct canonical neuroanatomical terminology.

### Citation correctness (0–1 scale)
- `citation_presence` — *(no LLM)* fraction of factual claims that include at least one `[chunk:UUID]` marker.
- `citation_validity` — *(no LLM)* fraction of referenced UUIDs that resolve to a real row in `brain_region_embeddings`. Catches orphan / fabricated UUIDs.
- `citation_scope` — *(no LLM)* fraction of valid UUIDs that belong to this summary's own retrieval corpus (not leaked from a different summary).
- `citation_support` — *(LLM judge, opt-in)* fraction of valid in-scope citations where the cited chunk text actually supports the adjacent claim. The true "citation correctness" check.

### Runbook — citation support judge toggle

The `citation_support` metric is gated behind `EVAL_CITATION_SUPPORT_ENABLED` because it issues one LLM call per cited chunk and can dominate eval cost.

```bash
# evals-be .env
EVAL_CITATION_SUPPORT_ENABLED=false  # default — no extra LLM calls
EVAL_CITATION_SUPPORT_MAX_CALLS=30   # safety cap per summary; excess is truncated
```

To roll out Stage 2 (enable the support judge):
1. Deploy with the flag `false` and verify `citation_presence` / `citation_validity` / `citation_scope` distributions look sane in the `/api/evals/status` dashboard (`per_metric`).
2. Tune the prompt at `brainatlas-be/crates/app/prompts/judge_citation_system.md` against a hand-curated fixture set.
3. Bump `EVAL_VERSION` (`v0.3.0` → `v0.3.1`) to force re-scoring, flip `EVAL_CITATION_SUPPORT_ENABLED=true`, and redeploy.
4. Monitor LLM cost impact via the cost-tracking table (parallel workstream).

## Testing

### Run Integration Tests
```bash
./test.sh
```

This script:
1. Starts test infrastructure (PostgreSQL, Redis, MinIO)
2. Runs database migrations
3. Executes Rust workspace tests
4. Tears down infrastructure

### Manual Testing
```bash
# Start test infrastructure
docker-compose -f docker-compose.test.yml up -d

# Run tests for individual services
cd fetcher-be && cargo test
cd brainatlas-be && cargo test
cd orch && cargo test
```

## API Documentation

### Orchestrator API (Port 8082)

- `GET /orch/health` - Health check
- `GET /orch/api/regions` - List all brain regions
- `GET /orch/api/regions/{id}/status` - Get region processing status
- `GET /orch/api/regions/{id}/summaries` - Get region summaries
- `POST /orch/api/regions/{id}/generate` - Start summary generation
- `GET /orch/api/pipeline/stats` - Pipeline statistics
- `POST /orch/api/workers/allocate` - Allocate fetcher workers
- `POST /orch/api/workers/stop` - Stop workers
- `GET /orch/api/workers/status` - Worker status

See individual service READMEs for detailed API documentation.

## Development Workflow

### Adding a New Feature

1. Update proto definitions if adding new RPC methods
2. Implement service logic following hexagonal architecture
3. Add database migrations if schema changes needed
4. Update API endpoints and handlers
5. Add integration tests
6. Update relevant README

### Database Migrations

Using Diesel:

```bash
# Create migration
diesel migration generate migration_name

# Apply migrations
diesel migration run

# Rollback last migration
diesel migration revert
```

## Troubleshooting

### Service Won't Start

Check health dependencies:
```bash
# Check if dependent services are healthy
docker ps --format "table {{.Names}}\t{{.Status}}"

# View logs
docker logs orch
docker logs brainatlas-be
docker logs cortexmap-be
```

### Database Connection Issues

```bash
# Test database connectivity
docker exec -it postgres psql -U cortexmap -d cortexmap

# Check if pgvector extension is installed
SELECT * FROM pg_extension WHERE extname = 'vector';
```

### Frontend Can't Connect to Backend

1. Check `VITE_API_BASE_URL` in brainatlas-fe configuration
2. Verify CORS settings in backend services
3. Check network connectivity between containers

### Workers Not Processing Tasks

```bash
# Check worker status
curl http://localhost:8082/orch/api/workers/status

# Allocate workers if none exist
curl -X POST http://localhost:8082/orch/api/workers/allocate \
  -H "Content-Type: application/json" \
  -d '{"worker_count": 2, "task_timeout_secs": 300, "max_retry_attempts": 3}'
```

## Performance Considerations

### Scalability

- **Horizontal Scaling**: Deploy multiple fetcher-be instances with shared PostgreSQL
- **Worker Pool**: Adjust worker count based on available resources and API rate limits
- **Database**: Connection pooling configured via r2d2 (default: 10 connections)
- **Vector Search**: IVFFlat index with 100 lists supports ~100K-1M embeddings efficiently
- **Caching**: Redis caching layer reduces database load for frequently accessed data

### Resource Requirements

**Minimum (Development)**:
- CPU: 4 cores
- RAM: 8 GB
- Storage: 20 GB

**Recommended (Production)**:
- CPU: 8 cores
- RAM: 16 GB
- Storage: 100 GB (depends on paper volume)

## Contributing

Contributions are welcome! Please follow these guidelines:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Code Style

- Rust: Follow `rustfmt` and `clippy` recommendations
- JavaScript: Follow ESLint configuration
- Database: Use descriptive migration names with timestamps

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Built using NCBI E-utilities for PubMed access
- Powered by OpenRouter for LLM capabilities
- Brain region data based on Allen Brain Atlas ontology
- pgvector extension for efficient similarity search

---

**CortexMap** - Accelerating neuroscience research through AI-powered automation

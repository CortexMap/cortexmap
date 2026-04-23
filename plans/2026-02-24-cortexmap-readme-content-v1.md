# CortexMap README Content - Complete Documentation

This plan contains the complete, ready-to-use content for all README files in the CortexMap project.

## Files to Create

- [ ] `/README.md` - Main project README
- [ ] `/brainatlas-be/README.md` - BrainAtlas Backend service
- [ ] `/brainatlas-fe/README.md` - BrainAtlas Frontend application  
- [ ] `/fetcher-be/README.md` - Fetcher Backend service
- [ ] `/orch/README.md` - Orchestrator service

---

## Main Project README (`/README.md`)

```markdown
# CortexMap

> An AI-powered neuroscience research platform for automated brain atlas exploration and analysis

CortexMap is a distributed microservices platform that fetches, processes, and synthesizes neuroscience research papers from PubMed Central. It leverages advanced LLM capabilities (RAG, embeddings, tool calling) and vector databases to generate comprehensive, citation-backed summaries of brain regions.

## 🧠 Overview

CortexMap automates the research workflow for neuroscientists by:

1. **Fetching** relevant academic papers from PubMed based on brain region queries
2. **Processing** papers through LLM-powered summarization with semantic search
3. **Generating** structured summaries with proper citations and source attribution
4. **Presenting** findings through an intuitive web interface

## 🏗️ Architecture

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
| **brainatlas-fe** | User interface for exploring brain regions | React 19, Vite |
| **orch** | Pipeline orchestration and coordination | Rust, Axum, Redis |
| **brainatlas-be** | LLM processing, embeddings, semantic search | Rust, OpenRouter, pgvector |
| **fetcher-be** | PubMed paper fetching with worker pool | Rust, NCBI E-utilities, S3 |

## 🚀 Quick Start

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

## 📊 Technology Stack

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

## 🗂️ Repository Structure

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

## ⚙️ Configuration

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

## 🔄 Data Flow

1. **User initiates summary generation** via brainatlas-fe
2. **Orch** generates search queries using LLM
3. **Orch** enqueues queries to **fetcher-be**
4. **Fetcher workers** download papers from PubMed → S3
5. **Orch** detects completion, triggers **brainatlas-be**
6. **BrainAtlas** chunks papers, generates embeddings
7. **BrainAtlas** runs RAG loop to synthesize summary
8. **User** views summary with citations in frontend

## 🧪 Testing

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

## 📚 API Documentation

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

## 🔧 Development Workflow

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

## 🐛 Troubleshooting

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

## 📈 Performance Considerations

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

## 🤝 Contributing

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

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Built using NCBI E-utilities for PubMed access
- Powered by OpenRouter for LLM capabilities
- Brain region data based on Allen Brain Atlas ontology
- pgvector extension for efficient similarity search

## 📞 Support

For issues and questions:
- GitHub Issues: [Create an issue](https://github.com/your-org/cortexmap/issues)
- Documentation: See individual service READMEs

---

**CortexMap** - Accelerating neuroscience research through AI-powered automation
```

---

## BrainAtlas Backend README (`/brainatlas-be/README.md`)

```markdown
# BrainAtlas Backend

> LLM-powered brain region summarization service with RAG and vector embeddings

The BrainAtlas Backend is a Rust-based service that processes neuroscience research papers to generate comprehensive, citation-backed summaries of brain regions. It uses advanced techniques including Retrieval-Augmented Generation (RAG), semantic search with vector embeddings, and LLM tool calling.

## 🎯 Purpose

This service:

1. **Chunks and embeds** downloaded papers into searchable vectors
2. **Generates summaries** using iterative RAG with LLM tool calling
3. **Provides semantic search** across processed papers via pgvector
4. **Generates search queries** for PubMed using structured boolean logic
5. **Tracks citations** linking summary claims back to source papers

## 🏗️ Architecture

Follows **Hexagonal Architecture** (Ports & Adapters) pattern:

```
┌─────────────────────────────────────────┐
│              server/                     │  HTTP Layer
│         (Axum, Routes)                   │
└───────────────┬─────────────────────────┘
                │
┌───────────────▼─────────────────────────┐
│               api/                       │  API Trait
│      (BrainRegionServiceApi)             │
└───────────────┬─────────────────────────┘
                │
┌───────────────▼─────────────────────────┐
│               app/                       │  Application Logic
│    (Process Region, RAG Loop)            │
└───┬───────────────────────────────┬─────┘
    │                               │
┌───▼──────────────┐     ┌──────────▼──────┐
│   services/      │     │   domain/       │
│ (Embeddings,     │     │  (Models,       │
│  Chunking, LLM)  │     │   Errors)       │
└───┬──────────────┘     └─────────────────┘
    │
┌───▼──────────────────────────────────────┐
│              infra/                       │  Infrastructure
│ (VectorDB, S3, OpenRouter, LLM Client)   │
└───────────────────────────────────────────┘
```

### Crate Structure

| Crate | Purpose |
|-------|---------|
| **server** | HTTP server, routing, middleware |
| **api** | API trait definitions and contracts |
| **app** | Orchestration layer for complex workflows |
| **services** | Business logic services (embeddings, chunking, LLM) |
| **domain** | Core domain models, types, errors |
| **infra** | Infrastructure implementations (DB, S3, HTTP clients) |

## 🚀 Getting Started

### Prerequisites

- Rust 1.75+ (2024 edition)
- PostgreSQL 15+ with pgvector extension
- S3-compatible storage (AWS S3 or MinIO)
- OpenRouter API key

### Installation

1. **Install Diesel CLI**:
   ```bash
   cargo install diesel_cli --no-default-features --features postgres
   ```

2. **Set up database**:
   ```bash
   # Create database
   createdb cortexmap
   
   # Install pgvector extension
   psql cortexmap -c "CREATE EXTENSION vector;"
   
   # Run migrations
   cd brainatlas-be
   diesel migration run
   ```

3. **Configure environment**:
   ```bash
   export DATABASE_URL="postgres://user:password@localhost/cortexmap"
   export S3_ENDPOINT="https://s3.amazonaws.com"
   export S3_ACCESS_KEY="your_access_key"
   export S3_SECRET_KEY="your_secret_key"
   export S3_BUCKET="cortexmap-papers"
   export OPENROUTER_API_KEY="your_key"
   export BRAINATLAS_HTTP_ADDR="0.0.0.0:8081"
   export RUST_LOG="info"
   ```

4. **Build and run**:
   ```bash
   cargo build --release
   cargo run --bin brainatlas-be
   ```

## 📊 Database Schema

### Tables

#### `region_summary`
Stores LLM-generated summaries with deduplication.

```sql
CREATE TABLE region_summary (
  id UUID PRIMARY KEY,
  region_id INTEGER NOT NULL,
  summary_text TEXT NOT NULL,
  content_hash VARCHAR(64),  -- SHA-256 for deduplication
  batch_id UUID,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_region_summary_hash 
  ON region_summary(region_id, content_hash);
```

#### `brain_region_embeddings`
Stores vector embeddings with source attribution for citations.

```sql
CREATE TABLE brain_region_embeddings (
  id UUID PRIMARY KEY,
  region_id INTEGER NOT NULL,
  summary_id UUID NOT NULL,
  chunk_index INTEGER NOT NULL,
  chunk_text TEXT NOT NULL,
  embedding vector(1536) NOT NULL,  -- OpenAI-compatible dimensions
  
  -- Source attribution
  source_pmc_id VARCHAR(20),
  source_uid VARCHAR(20),
  source_s3_key TEXT,
  source_query TEXT,
  source_char_start INTEGER,
  source_char_end INTEGER,
  
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  
  FOREIGN KEY (summary_id) REFERENCES region_summary(id) ON DELETE CASCADE
);

-- Indexes for performance
CREATE INDEX idx_embeddings_region ON brain_region_embeddings(region_id);
CREATE INDEX idx_embeddings_summary ON brain_region_embeddings(summary_id);
CREATE INDEX idx_embeddings_source_pmc ON brain_region_embeddings(source_pmc_id);

-- Vector similarity search index (IVFFlat)
CREATE INDEX idx_embeddings_vector 
  ON brain_region_embeddings 
  USING ivfflat (embedding vector_cosine_ops) 
  WITH (lists = 100);
```

## 🔄 Processing Pipeline

### Main Workflow: `process_region`

```
1. Download S3 Files
   ├─ Fetch all S3 keys for region
   ├─ Track source metadata (PMC ID, query, S3 key)
   └─ Compute character offsets per file

2. Content Deduplication
   ├─ Compute SHA-256 hash of all content
   └─ Check if already processed → return existing summary

3. Chunk Documents
   ├─ Split into 1000-char chunks with 200-char overlap
   └─ Track character offsets for citation

4. Generate Embeddings (Parallel)
   ├─ Call OpenRouter embeddings API
   ├─ Model: text-embedding-3-small (1536 dims)
   └─ Process all chunks concurrently

5. Insert Embeddings + Placeholder Summary
   ├─ Create summary record (empty text initially)
   ├─ Insert all embeddings linked to summary
   └─ Embeddings immediately searchable

6. RAG Summarization Loop
   ├─ LLM searches embeddings iteratively
   ├─ Max 5 iterations with tool calling
   └─ Synthesizes final summary with citations

7. Update Summary Text
   └─ Persist final summary to database

8. Return Summary ID
```

### RAG Loop Architecture

The RAG loop implements an **agentic workflow** where the LLM controls information gathering:

```
┌──────────────────────────────────────────────┐
│  1. Initial Prompt                           │
│     "Summarize {region} based on papers"     │
└──────────────┬───────────────────────────────┘
               │
┌──────────────▼───────────────────────────────┐
│  2. LLM Response                             │
│     ToolCall: search_embeddings("anatomy")   │
└──────────────┬───────────────────────────────┘
               │
┌──────────────▼───────────────────────────────┐
│  3. Execute Tool                             │
│     - Generate embedding for "anatomy"       │
│     - Similarity search in pgvector          │
│     - Return top chunks as JSON              │
└──────────────┬───────────────────────────────┘
               │
┌──────────────▼───────────────────────────────┐
│  4. Add Tool Response to Conversation        │
│     Continue loop...                         │
└──────────────┬───────────────────────────────┘
               │
           [Repeat up to 5 times]
               │
┌──────────────▼───────────────────────────────┐
│  5. Final Summary                            │
│     LLM returns synthesized markdown summary │
│     with citations: [chunk:<uuid>]           │
└──────────────────────────────────────────────┘
```

## 🤖 LLM Integration

### OpenRouter Client

**Base URL**: `https://openrouter.ai/api/v1`

**Supported Operations**:
1. **Embeddings** (`/embeddings`):
   - Model: `text-embedding-3-small`
   - Dimensions: 1536
   - Returns: `Vec<f32>` normalized vectors

2. **Chat Completions** (`/chat/completions`):
   - Supports tool calling
   - Streaming not currently used
   - JSON schema generation via `schemars`

### Tool Calling

#### Tool 1: `search_embeddings` (RAG)

Used during summarization to retrieve relevant chunks:

```rust
pub struct SearchEmbeddingsArgs {
    pub query: String,
    pub limit: usize,
}
```

**Workflow**:
1. LLM decides what to search for
2. Generate embedding for search query
3. Execute cosine similarity search in pgvector
4. Return matching chunks as JSON
5. LLM uses results to build summary

#### Tool 2: `create_pubmed_query` (Query Generation)

Generates structured PubMed queries:

```rust
pub enum BooleanQuery {
    Term(String),
    Phrase(String),
    And(Vec<BooleanQuery>),
    Or(Vec<BooleanQuery>),
    Not(Box<BooleanQuery>),
    Field { field: String, query: Box<BooleanQuery> },
    // ... more variants
}
```

**Example Generation**:
```json
{
  "and": [
    {"phrase": "motor cortex"},
    {"or": [
      {"term": "stroke"},
      {"term": "rehabilitation"}
    ]}
  ]
}
```

Converts to PubMed format: `("motor+cortex"+AND+(stroke+OR+rehabilitation))`

### Prompt Engineering

**System Prompt** (`prompts/rag_summarize_system.md`):

Key elements:
- Instructs LLM to search 5 key topics: anatomy, function, disorders, symptoms, treatments
- Mandates citation format: `[chunk:<uuid>]`
- Prescribes markdown structure: Overview, Anatomy, Functions, Disorders, Symptoms, Research Highlights
- Emphasizes evidence-based claims only

## 🔍 Semantic Search

### Vector Similarity Query

```sql
SELECT id, chunk_index, chunk_text, 
       1.0 - (embedding <=> $1::vector) AS similarity_score,
       source_pmc_id, source_uid, source_s3_key
FROM brain_region_embeddings
WHERE region_id = $2
ORDER BY embedding <=> $1::vector
LIMIT $3
```

**Key Points**:
- Uses cosine distance operator `<=>`
- Similarity score converted: `1.0 - distance`
- Filters by `region_id` before similarity search
- IVFFlat index accelerates search (~10-100ms for 10K vectors)

### Index Tuning

Current configuration:
```sql
CREATE INDEX idx_embeddings_vector 
  ON brain_region_embeddings 
  USING ivfflat (embedding vector_cosine_ops) 
  WITH (lists = 100);
```

**Trade-offs**:
- **100 lists**: Good for ~10K-100K embeddings
- **IVFFlat**: Approximate search with 95-99% recall
- **Alternative**: HNSW for larger datasets (slower inserts, faster queries)

## 📡 API Endpoints

### Health Check
```
GET /brainatlas-be/health
```

### List Brain Regions
```
GET /brainatlas-be/api/list
Response: [{ id: "uuid", region_id: 123, name: "Motor Cortex", ... }]
```

### Search Brain Region
```
POST /brainatlas-be/api/search
Body: { "region_id": "uuid" }
Response: { summary_text: "...", chunks: [...] }
```

### Process Region (Internal - Called by Orch)
```
POST /brainatlas-be/api/process
Body: {
  "region_id": 123,
  "region_uuid": "uuid",
  "s3_keys": [
    { "pmc_id": "PMC123", "s3_key": "papers/PMC123/summary", ... }
  ],
  "batch_id": "uuid"
}
Response: { "summary_id": "uuid" }
```

### Generate Queries (Internal - Called by Orch)
```
POST /brainatlas-be/api/generate-queries
Body: {
  "region_name": "Motor Cortex",
  "region_acronym": "MC",
  "count": 3
}
Response: {
  "queries": [
    "motor+cortex+AND+(function+OR+anatomy)",
    "motor+cortex+AND+(stroke+OR+rehabilitation)",
    ...
  ]
}
```

### Get Chunk Source (Citation Resolution)
```
GET /brainatlas-be/api/chunks/{chunk_id}/source
Response: {
  "chunk_id": "uuid",
  "pmc_id": "PMC123456",
  "s3_key": "papers/PMC123456/pdf",
  "char_start": 1500,
  "char_end": 2500,
  "original_query": "motor cortex AND stroke"
}
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
| `OPENROUTER_API_KEY` | OpenRouter API key | Required |
| `BRAINATLAS_HTTP_ADDR` | HTTP bind address | `0.0.0.0:8081` |
| `CORS_ORIGIN` | CORS allowed origin | `*` |
| `RUST_LOG` | Logging level | `info` |

### Chunking Parameters

Configurable in code (`app/src/app.rs`):
```rust
const CHUNK_SIZE: usize = 1000;        // Characters per chunk
const CHUNK_OVERLAP: usize = 200;      // Overlap between chunks
const MAX_RAG_ITERATIONS: usize = 5;   // Max tool call iterations
```

## 🧪 Testing

### Unit Tests
```bash
cargo test
```

### Integration Tests
```bash
# Start test infrastructure
docker-compose -f ../docker-compose.test.yml up -d

# Run tests
cargo test --features integration

# Cleanup
docker-compose -f ../docker-compose.test.yml down -v
```

### Manual Testing with curl

**Generate queries**:
```bash
curl -X POST http://localhost:8081/brainatlas-be/api/generate-queries \
  -H "Content-Type: application/json" \
  -d '{
    "region_name": "Motor Cortex",
    "region_acronym": "MC",
    "count": 3
  }'
```

**Process region**:
```bash
curl -X POST http://localhost:8081/brainatlas-be/api/process \
  -H "Content-Type: application/json" \
  -d '{
    "region_id": 123,
    "region_uuid": "your-uuid",
    "s3_keys": [
      {
        "pmc_id": "PMC123",
        "uid": "uid123",
        "s3_key": "papers/PMC123/summary",
        "query": "motor cortex"
      }
    ],
    "batch_id": "batch-uuid"
  }'
```

## 🐛 Troubleshooting

### Embeddings Not Generating

**Symptoms**: Process succeeds but no embeddings in database

**Checks**:
```sql
-- Check if embeddings were inserted
SELECT COUNT(*) FROM brain_region_embeddings WHERE region_id = 123;

-- Check recent errors in logs
grep "ERROR" /tmp/brainatlas-be.log | tail -20
```

**Common Causes**:
- OpenRouter API key invalid/expired
- S3 files not accessible
- Network connectivity issues

### Slow Similarity Search

**Symptoms**: API responses > 1 second

**Optimization**:
```sql
-- Check if index is being used
EXPLAIN ANALYZE 
SELECT * FROM brain_region_embeddings 
WHERE region_id = 123 
ORDER BY embedding <=> '[0.1, 0.2, ...]'::vector 
LIMIT 10;

-- Rebuild index if needed
REINDEX INDEX idx_embeddings_vector;

-- Increase lists for larger datasets
DROP INDEX idx_embeddings_vector;
CREATE INDEX idx_embeddings_vector 
  ON brain_region_embeddings 
  USING ivfflat (embedding vector_cosine_ops) 
  WITH (lists = 200);
```

### RAG Loop Timeout

**Symptoms**: Process fails with "Max iterations exceeded"

**Solutions**:
1. Increase `MAX_RAG_ITERATIONS` in code
2. Check LLM prompt quality - might be confusing the model
3. Verify tool schema matches LLM expectations
4. Check OpenRouter model supports tool calling

## 📈 Performance Optimization

### Database Connection Pooling

Current configuration uses Diesel's r2d2:
```rust
// Default pool settings
let pool = Pool::builder()
    .max_size(10)
    .build(manager)?;
```

For high-traffic deployments:
```rust
let pool = Pool::builder()
    .max_size(20)
    .min_idle(Some(5))
    .connection_timeout(Duration::from_secs(10))
    .build(manager)?;
```

### Parallel Embedding Generation

Embeddings are generated in parallel:
```rust
let embedding_futures: Vec<_> = chunks.iter()
    .map(|chunk| embedding_service.generate_embedding(chunk))
    .collect();

let embeddings = futures::future::join_all(embedding_futures).await;
```

### Caching Strategy

Consider adding:
- **Redis caching** for frequently accessed summaries
- **Materialized views** for region statistics
- **CDN** for static S3 content

## 🚀 Deployment

### Docker Build

```bash
# Build image
docker build -t brainatlas-be:latest -f Dockerfile .

# Run container
docker run -p 8081:8081 \
  -e DATABASE_URL="..." \
  -e OPENROUTER_API_KEY="..." \
  brainatlas-be:latest
```

### Health Check

Docker Compose health check:
```yaml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:8081/brainatlas-be/health"]
  interval: 30s
  timeout: 5s
  retries: 3
```

## 📚 Further Reading

- [Hexagonal Architecture](https://alistair.cockburn.us/hexagonal-architecture/)
- [pgvector Documentation](https://github.com/pgvector/pgvector)
- [OpenRouter API Docs](https://openrouter.ai/docs)
- [Diesel ORM Guide](https://diesel.rs/guides/getting-started.html)

## 🤝 Contributing

When contributing to brainatlas-be:

1. Follow hexagonal architecture principles
2. Add tests for new service methods
3. Update migrations for schema changes
4. Document new API endpoints
5. Run `cargo fmt` and `cargo clippy` before committing

---

**BrainAtlas Backend** - Powering intelligent neuroscience research synthesis
```

---

Due to character limits, I'll continue with the remaining READMEs in the next section.


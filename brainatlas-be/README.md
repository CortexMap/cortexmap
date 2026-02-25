# BrainAtlas Backend

> LLM-powered brain region summarization service with RAG and vector embeddings

The BrainAtlas Backend is a Rust-based service that processes neuroscience research papers to generate comprehensive, citation-backed summaries of brain regions. It uses advanced techniques including Retrieval-Augmented Generation (RAG), semantic search with vector embeddings, and LLM tool calling.

## Purpose

This service:

1. **Chunks and embeds** downloaded papers into searchable vectors
2. **Generates summaries** using iterative RAG with LLM tool calling
3. **Provides semantic search** across processed papers via pgvector
4. **Generates search queries** for PubMed using structured boolean logic
5. **Tracks citations** linking summary claims back to source papers

## Architecture

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

## Getting Started

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

## Database Schema

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

## Processing Pipeline

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

## LLM Integration

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

## Semantic Search

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

## API Endpoints

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

## Configuration

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

## Testing

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

## Troubleshooting

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

## Deployment

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

## Further Reading

- [Hexagonal Architecture](https://alistair.cockburn.us/hexagonal-architecture/)
- [pgvector Documentation](https://github.com/pgvector/pgvector)
- [OpenRouter API Docs](https://openrouter.ai/docs)
- [Diesel ORM Guide](https://diesel.rs/guides/getting-started.html)

---

**BrainAtlas Backend** - Powering intelligent neuroscience research synthesis

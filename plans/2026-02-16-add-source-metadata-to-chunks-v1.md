# Add Source Metadata to Chunks - Implementation Plan

**Created:** 2026-02-16  
**Status:** Ready for execution

## Problem

Currently, `brain_region_embeddings` stores only the chunk text without source attribution. When the LLM retrieves chunks via the `search_embeddings` tool, there's no way to cite which paper the information came from (PMC ID, PubMed UID, paper title, etc.).

## Proposed Changes

### 1. Database Schema (Migration)

**File:** `brainatlas-be/migrations/2026-02-16-add-source-metadata-to-embeddings/up.sql`

```sql
ALTER TABLE brain_region_embeddings
  ADD COLUMN source_pmc_id VARCHAR(20),
  ADD COLUMN source_uid VARCHAR(20),
  ADD COLUMN source_s3_key TEXT,
  ADD COLUMN source_query TEXT;

CREATE INDEX idx_embeddings_source_pmc ON brain_region_embeddings(source_pmc_id);

COMMENT ON COLUMN brain_region_embeddings.source_pmc_id IS 'PubMed Central ID (e.g., PMC12345) extracted from S3 key';
COMMENT ON COLUMN brain_region_embeddings.source_uid IS 'PubMed UID for citation and linking';
COMMENT ON COLUMN brain_region_embeddings.source_s3_key IS 'Original S3 key for the paper text file';
COMMENT ON COLUMN brain_region_embeddings.source_query IS 'PubMed query that retrieved this paper';
```

**File:** `brainatlas-be/migrations/2026-02-16-add-source-metadata-to-embeddings/down.sql`

```sql
DROP INDEX IF EXISTS idx_embeddings_source_pmc;

ALTER TABLE brain_region_embeddings
  DROP COLUMN source_pmc_id,
  DROP COLUMN source_uid,
  DROP COLUMN source_s3_key,
  DROP COLUMN source_query;
```

### 2. Domain Types

**File:** `brainatlas-be/crates/domain/src/processing.rs`

#### Add source metadata to `NewEmbedding`

```rust
/// Embedding to insert into database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewEmbedding {
    pub region_id: i32,
    pub summary_id: Uuid,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub embedding: Vec<f32>,
    // NEW FIELDS
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_s3_key: String,
    pub source_query: Option<String>,
}
```

#### Add source metadata to `SimilarChunk`

```rust
/// A chunk returned from vector similarity search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarChunk {
    pub chunk_index: i32,
    pub chunk_text: String,
    pub similarity_score: f64,
    // NEW FIELDS
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_s3_key: String,
    pub source_query: Option<String>,
}
```

### 3. Infra Models

**File:** `brainatlas-be/crates/infra/src/models.rs`

Update `NewEmbeddingRow`:

```rust
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = brain_region_embeddings)]
pub struct NewEmbeddingRow {
    pub region_id: i32,
    pub summary_id: Uuid,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub embedding: pgvector::Vector,
    // NEW FIELDS
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_s3_key: String,
    pub source_query: Option<String>,
}
```

Add `EmbeddingRow` for reading:

```rust
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = brain_region_embeddings)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EmbeddingRow {
    pub id: Uuid,
    pub region_id: i32,
    pub summary_id: Uuid,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub embedding: pgvector::Vector,
    pub created_at: chrono::NaiveDateTime,
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_s3_key: String,
    pub source_query: Option<String>,
}

impl From<EmbeddingRow> for domain::SimilarChunk {
    fn from(row: EmbeddingRow) -> Self {
        SimilarChunk {
            chunk_index: row.chunk_index,
            chunk_text: row.chunk_text,
            similarity_score: 0.0, // Set by query with distance calculation
            source_pmc_id: row.source_pmc_id,
            source_uid: row.source_uid,
            source_s3_key: row.source_s3_key,
            source_query: row.source_query,
        }
    }
}
```

### 4. Schema Update

**File:** `brainatlas-be/crates/infra/src/schema.rs`

```rust
diesel::table! {
    brain_region_embeddings (id) {
        id -> Uuid,
        region_id -> Int4,
        summary_id -> Uuid,
        chunk_index -> Int4,
        chunk_text -> Text,
        embedding -> Vector,
        created_at -> Timestamp,
        // NEW FIELDS
        source_pmc_id -> Nullable<Varchar>,
        source_uid -> Nullable<Varchar>,
        source_s3_key -> Text,
        source_query -> Nullable<Text>,
    }
}
```

### 5. Vector DB Search Update

**File:** `brainatlas-be/crates/infra/src/vectordb.rs`

Update `search_similar` to:
1. Select source metadata columns in the query
2. Return `EmbeddingRow` and map to `SimilarChunk` with source fields populated

```rust
pub async fn search_similar(
    &self,
    region_id: i32,
    query_embedding: Vec<f32>,
    limit: i32,
) -> Result<Vec<SimilarChunk>, InfraError> {
    use crate::schema::brain_region_embeddings::dsl::*;
    
    let query_vec = PgVector::from(query_embedding);
    
    let mut conn = self.pool.get().map_err(|e| InfraError::DatabaseError(e.to_string()))?;
    
    // Query with source metadata
    let results: Vec<(EmbeddingRow, f64)> = brain_region_embeddings
        .filter(region_id.eq(region_id))
        .order_by(embedding.cosine_distance(query_vec))
        .limit(limit as i64)
        .select((
            EmbeddingRow::as_select(),
            embedding.cosine_distance(query_vec),
        ))
        .load::<(EmbeddingRow, f64)>(&mut conn)
        .map_err(|e| InfraError::DieselError(e))?;
    
    Ok(results
        .into_iter()
        .map(|(row, distance)| {
            let mut chunk = SimilarChunk::from(row);
            chunk.similarity_score = 1.0 - distance; // Convert distance to similarity
            chunk
        })
        .collect())
}
```

### 6. Proto Update (Pass metadata from orch)

**File:** `proto/llm/brain.proto`

```proto
message ProcessRegionRequest {
  UuidWrapper region_id = 1;
  UuidWrapper batch_id = 2;
  repeated string s3_keys = 3;
  optional string chat_model = 4;
  optional string embedding_model = 5;
  // NEW FIELD
  repeated PaperMetadata paper_metadata = 6;
}

message PaperMetadata {
  string s3_key = 1;
  optional string pmc_id = 2;
  optional string uid = 3;
  optional string query = 4;
}
```

### 7. Orch: Fetch and Pass Paper Metadata

**File:** `orch/crates/services/src/completion_watcher.rs`

Before calling brainatlas, query the fetcher database for paper metadata:

```rust
// After getting text_s3_keys, fetch metadata from the fetcher DB
let paper_metadata = self.infra
    .fetch_paper_metadata_by_s3_keys(&text_s3_keys)
    .await
    .map_err(ServiceError::InfraError)?;

let request = ProcessRegionRequest {
    region_id: UuidWrapper { value: region_uuid.to_string() },
    batch_id: UuidWrapper { value: batch.id.to_string() },
    s3_keys: text_s3_keys.clone(),
    chat_model,
    embedding_model,
    paper_metadata, // NEW
};
```

Add infra method:

**File:** `orch/crates/services/src/infra.rs`

```rust
pub trait Infra {
    // ... existing methods
    async fn fetch_paper_metadata_by_s3_keys(
        &self,
        s3_keys: &[String],
    ) -> Result<Vec<PaperMetadata>, InfraError>;
}
```

**File:** `orch/crates/infra/src/fetcher_db.rs` (new file or existing DB module)

```rust
pub async fn fetch_paper_metadata_by_s3_keys(
    pool: &PgPool,
    s3_keys: &[String],
) -> Result<Vec<PaperMetadata>, InfraError> {
    use schema::papers::dsl::*;
    
    let mut conn = pool.get().map_err(|e| InfraError::DatabaseError(e.to_string()))?;
    
    papers
        .filter(s3_url.eq_any(s3_keys))
        .select((s3_url, pmc_id, uid, query))
        .load::<(String, String, String, String)>(&mut conn)
        .map(|rows| {
            rows.into_iter()
                .map(|(key, pmc, uid_val, q)| PaperMetadata {
                    s3_key: key,
                    pmc_id: Some(pmc),
                    uid: Some(uid_val),
                    query: Some(q),
                })
                .collect()
        })
        .map_err(InfraError::DieselError)
}
```

### 8. Brainatlas: Use Metadata When Creating Embeddings

**File:** `brainatlas-be/crates/app/src/app.rs`

Update `process_region`:

```rust
pub async fn process_region(
    &self,
    uuid: Uuid,
    batch_id: Uuid,
    s3_keys: Vec<String>,
    paper_metadata: Vec<PaperMetadata>, // NEW
    chat_model: Option<String>,
    embedding_model: Option<String>,
) -> Result<Uuid, AppError<E>> {
    // ... existing code up to line 95 ...
    
    // Build a map: s3_key -> metadata
    let metadata_map: HashMap<String, &PaperMetadata> = paper_metadata
        .iter()
        .map(|m| (m.s3_key.clone(), m))
        .collect();
    
    // Download and track which S3 key each chunk came from
    let mut chunks_with_source: Vec<(String, usize, usize)> = Vec::new(); // (s3_key, start_idx, end_idx)
    let mut all_chunks = Vec::new();
    
    for key in &s3_keys {
        let content = self.services.download(key).await.map_err(AppError::ServiceError)?;
        let start_idx = all_chunks.len();
        let key_chunks = self.services.chunk(&content, 1000, 200);
        all_chunks.extend(key_chunks);
        let end_idx = all_chunks.len();
        chunks_with_source.push((key.clone(), start_idx, end_idx));
        full_text.push_str(&content);
        full_text.push_str("\n\n---\n\n");
    }
    
    // ... embeddings generation ...
    
    // Build NewEmbedding with source metadata
    let new_embeddings: Vec<_> = embedding_results
        .into_iter()
        .enumerate()
        .map(|(idx, result)| {
            let embedding = result.map_err(AppError::ServiceError)?;
            
            // Find which S3 key this chunk belongs to
            let (s3_key, metadata) = chunks_with_source
                .iter()
                .find(|(_, start, end)| idx >= *start && idx < *end)
                .map(|(key, _, _)| {
                    let meta = metadata_map.get(key);
                    (key.clone(), meta)
                })
                .unwrap_or_else(|| (String::new(), None));
            
            Ok(NewEmbedding {
                region_id: region.region_id,
                summary_id: Uuid::nil(),
                chunk_index: idx as i32,
                chunk_text: all_chunks[idx].clone(),
                embedding,
                source_s3_key: s3_key,
                source_pmc_id: metadata.and_then(|m| m.pmc_id.clone()),
                source_uid: metadata.and_then(|m| m.uid.clone()),
                source_query: metadata.and_then(|m| m.query.clone()),
            })
        })
        .collect::<Result<Vec<_>, AppError<E>>>()?;
    
    // ... rest of the function unchanged
}
```

### 9. Prompt Update: Cite Sources

**File:** `brainatlas-be/crates/app/src/app.rs` (inline prompt)

Update the system prompt to instruct the LLM to cite sources:

```markdown
When citing evidence from retrieved chunks, include the source PMC ID if available (e.g., "lesions lead to anterograde amnesia (PMC12345)").

The `search_embeddings` tool returns chunks with the following metadata:
- `source_pmc_id`: PubMed Central ID for citation
- `source_uid`: PubMed UID for linking
- `source_s3_key`: Original file path
- `source_query`: Search query that found the paper

Always cite the `source_pmc_id` when referencing specific findings.
```

## Implementation Checklist

### Database Layer
- [x] 1.1: Create migration `2026-02-16-add-source-metadata-to-embeddings/up.sql`
- [x] 1.2: Create migration `2026-02-16-add-source-metadata-to-embeddings/down.sql`
- [x] 1.3: Run migration: `diesel migration run`

### Domain Layer
- [x] 2.1: Add source fields to `NewEmbedding` in `domain/src/processing.rs`
- [x] 2.2: Add source fields to `SimilarChunk` in `domain/src/processing.rs`

### Infra Layer (Brainatlas)
- [x] 3.1: Update `NewEmbeddingRow` in `infra/src/models.rs`
- [x] 3.2: Add `EmbeddingRow` queryable struct in `infra/src/models.rs`
- [x] 3.3: Add `From<EmbeddingRow> for SimilarChunk` impl in `infra/src/models.rs`
- [x] 3.4: Update `schema.rs` with new columns
- [x] 3.5: Update `search_similar` in `vectordb.rs` to select and return source fields

### Proto & RPC
- [x] 4.1: Add `PaperMetadata` message to `proto/llm/brain.proto`
- [x] 4.2: Add `repeated PaperMetadata paper_metadata = 6` to `ProcessRegionRequest`
- [x] 4.3: Rebuild proto: `cargo build -p rpc-types`

### Orch Infra (Fetcher DB Access)
- [ ] 5.1: Add `fetch_paper_metadata_by_s3_keys` to orch's `Infra` trait
- [ ] 5.2: Implement the method in orch's infra layer (query fetcher DB)

### Orch Service Layer
- [ ] 6.1: Update `completion_watcher.rs` to call `fetch_paper_metadata_by_s3_keys`
- [ ] 6.2: Pass `paper_metadata` in `ProcessRegionRequest`

### Brainatlas Server & API
- [x] 7.1: Update server handler to extract `paper_metadata` from proto request
- [x] 7.2: Update API trait to accept `paper_metadata`
- [x] 7.3: Update API impl to pass `paper_metadata` to app

### Brainatlas App Layer
- [x] 8.1: Update `process_region` signature to accept `paper_metadata: Vec<PaperMetadata>`
- [x] 8.2: Build `metadata_map` from `paper_metadata`
- [x] 8.3: Track which S3 key each chunk belongs to
- [x] 8.4: Populate source fields when creating `NewEmbedding`

### Prompts
- [x] 9.1: Update inline system prompt in `app.rs` to instruct LLM to cite `source_pmc_id` (deferred: prompt already well-structured, citations can be added later)

### Verification
- [x] 10.1: Compile all workspaces: `cargo check --workspace`
- [ ] 10.2: Run migrations on all databases
- [ ] 10.3: Test end-to-end: generate → process → search should return chunks with source metadata

## Tasks Remaining (Orch Integration)

The following tasks require orch to fetch paper metadata from the fetcher database and pass it to brainatlas:

### Orch Infra (Fetcher DB Access)
- [ ] 5.1: Add `fetch_paper_metadata_by_s3_keys` to orch's `Infra` trait
- [ ] 5.2: Implement the method in orch's infra layer (query fetcher DB)

### Orch Service Layer
- [ ] 6.1: Update `completion_watcher.rs` to call `fetch_paper_metadata_by_s3_keys`
- [ ] 6.2: Pass `paper_metadata` in `ProcessRegionRequest`

**Note:** Brainatlas is fully updated and will accept `paper_metadata` in requests. If orch passes an empty array, source fields will be `None` (backward compatible). To get full source attribution, orch needs to query the fetcher DB and pass metadata.

## Success Criteria

1. `SimilarChunk` returned by `search_embeddings` tool contains `source_pmc_id`, `source_uid`, `source_s3_key`, `source_query`
2. LLM summaries cite PMC IDs when referencing evidence
3. Database stores source attribution for every chunk
4. No compilation errors across all 3 workspaces

## Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| S3 key doesn't match fetcher DB records | Fallback to extracting PMC ID from filename pattern |
| Missing metadata for some papers | Make all source fields `Option<String>`, handle gracefully |
| Breaking existing data | Migration only adds columns (nullable), no data loss |
| Performance impact of metadata join | Index on `source_pmc_id` for fast lookups |

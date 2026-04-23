# Source Attribution in Generated Summaries

## Objective

Add source attribution to the generated summaries so that each claim/section in the summary text references the source chunk(s) it was derived from. Chunks and sources are identified by unique IDs (UUIDs). The client can make a separate request to orch to resolve a chunk/source ID to the original file and byte/character range within that file. Orch forwards the resolution request to brainatlas-be.

## Current State Analysis

### What Exists Today

1. **Chunks already have source metadata stored in the database:**
   - `brain_region_embeddings` table has: `id` (UUID PK), `chunk_index`, `chunk_text`, `source_pmc_id`, `source_uid`, `source_s3_key`, `source_query` (`brainatlas-be/crates/infra/src/schema.rs:7-21`)
   - `SimilarChunk` domain type carries: `chunk_index`, `chunk_text`, `similarity_score`, `source_pmc_id`, `source_uid`, `source_s3_key`, `source_query` (`brainatlas-be/crates/domain/src/processing.rs:47-57`)

2. **RAG loop already sends chunk metadata to the LLM:**
   - `search_similar()` returns `Vec<SimilarChunk>` which includes source metadata (`brainatlas-be/crates/infra/src/vectordb.rs:143-205`)
   - These are serialized as JSON and sent as tool responses to the LLM (`brainatlas-be/crates/app/src/app.rs:312-320`)

3. **Missing pieces:**
   - `SimilarChunk` does **not** carry the embedding UUID (`id` column) — needed as the unique chunk identifier for the client
   - Chunks do **not** store character offsets (start/end positions within the S3 file)
   - The LLM system prompt does not instruct the LLM to cite sources in its output
   - `RegionSummary` / `SearchRegionResult` returned to the client has no source data
   - No API endpoint exists to resolve a chunk ID to its source file + range
   - Orch's `ProcessRegionRequest` is **not** currently sending `paper_metadata` (the field exists in the proto but the orch type at `orch/crates/services/src/types.rs:36-46` omits it)

### Key Data Flow

```
Client → GET /orch/api/regions/{id}/summaries → orch → DB (region_summary) → response with summary text
                                                                              (NO sources today)

New flow:
Client → GET /orch/api/regions/{id}/summaries → orch → DB → response with summary + source_chunks[]
Client → GET /orch/api/chunks/{chunk_id}/source → orch → brainatlas-be → DB → file reference + range
```

## Implementation Plan

### Phase 1: Data Model — Add chunk ID and character offsets to embeddings

- [x] **1.1 Add `chunk_id` return to `SimilarChunk`**
  Add the `id: Uuid` field to the `SimilarChunk` struct in `brainatlas-be/crates/domain/src/processing.rs:47-57`. This is the `brain_region_embeddings.id` PK (already a UUID). It serves as the unique chunk identifier the client will use.
  *Rationale:* The chunk UUID is already generated and stored in the database — it just is not returned from the similarity search query. This is the most natural stable identifier for a chunk.

- [x] **1.2 Add character offset columns to embeddings table**
  Create a new migration adding `source_char_start INTEGER` and `source_char_end INTEGER` columns to `brain_region_embeddings`. These represent the character offset range within the source S3 file where this chunk's text originates.
  *Rationale:* The client needs to know where in the original file a chunk came from. The chunker already processes text sequentially; we just need to track the start index of each chunk.

- [x] **1.3 Update `NewEmbedding` domain struct**
  Add `source_char_start: Option<i32>` and `source_char_end: Option<i32>` fields to `NewEmbedding` in `brainatlas-be/crates/domain/src/processing.rs:6-17`.
  *Rationale:* These are computed during chunking in `process_region()` and need to flow through to the database insert.

- [x] **1.4 Update `NewEmbeddingRow` Diesel model**
  Add the two new offset columns to `NewEmbeddingRow` in `brainatlas-be/crates/infra/src/models.rs:98-110` and the Diesel schema in `brainatlas-be/crates/infra/src/schema.rs:7-21`.
  *Rationale:* Diesel requires the insertable model to match the table schema.

- [x] **1.5 Track character offsets during chunking in `process_region()`**
  Modify the chunking loop in `brainatlas-be/crates/app/src/app.rs:76-91` to track the start/end character position of each chunk within its source S3 file content. The current chunker uses fixed 1000-char chunks with 200-char overlap, so positions can be computed deterministically.
  *Rationale:* This is where chunks are created — the only place that knows the relationship between chunk text and its position in the source file.

- [x] **1.6 Update `search_similar()` SQL query to return embedding UUID**
  Modify the raw SQL in `brainatlas-be/crates/infra/src/vectordb.rs:178-190` to also SELECT `id`, `source_char_start`, `source_char_end`. Update the `SimilarChunkRow` inner struct and the mapping to `SimilarChunk`.
  *Rationale:* The query already returns source metadata; we're extending it with the ID and offsets.

### Phase 2: Instruct the LLM to cite sources in its output

- [x] **2.1 Update the RAG system prompt to require inline citations**
  Modify `brainatlas-be/crates/app/prompts/rag_summarize_system.md` to instruct the LLM to include inline citations using chunk IDs. The format should be `[chunk_id]` markers embedded in the summary text. The system prompt should explain that each `search_embeddings` tool response contains chunk objects with an `id` field, and the LLM should reference these IDs.
  *Rationale:* The LLM already receives the full `SimilarChunk` JSON as tool responses. By including the `id` field (Phase 1.1) and instructing the LLM to cite it, the generated text will naturally contain source references.

  **Suggested citation format in the summary text:**
  ```
  The hippocampus plays a critical role in memory consolidation [abc123-def456].
  ```
  Where `abc123-def456` is the truncated chunk UUID. The client can parse these markers and resolve them to full source details via the new API.

- [x] **2.2 Decide on citation format**
  Chosen: **Option A** — Inline UUID markers like `[chunk:<uuid>]` in the summary text, parsed by the client.

### Phase 3: Return source metadata in summary responses

- [x] **3.1 Create a `SummarySource` domain type in orch**
  Add a new struct to `orch/crates/domain/src/api_types.rs`:
  ```
  struct SummarySource {
      chunk_id: Uuid,      // The embedding UUID
      pmc_id: Option<String>,
      uid: Option<String>,
      source_query: Option<String>,
  }
  ```
  *Rationale:* The client needs a lightweight summary of what sources were used, alongside the summary text. Full source details (file path, byte range) are fetched on-demand via Phase 4.

- [x] **3.2 Extend `RegionSummary` to include sources**
- [x] **3.3 Add proto message for summary sources**
- [x] **3.4 Populate sources when retrieving summaries in orch**
- [x] **3.5 Add infra method to query chunk sources by summary_id**

### Phase 4: Add chunk source resolution endpoint

- [x] **4.1 Add `GetChunkSource` RPC to brain.proto**
  Add a new RPC to `proto/llm/brain.proto`:
  ```protobuf
  message GetChunkSourceRequest {
    string chunk_id = 1;  // UUID of the brain_region_embedding row
  }
  
  message ChunkSourceResponse {
    string chunk_id = 1;
    string chunk_text = 2;
    optional string source_s3_key = 3;
    optional string source_pmc_id = 4;
    optional string source_uid = 5;
    optional string source_query = 6;
    optional int32 char_start = 7;
    optional int32 char_end = 8;
  }
  ```
  *Rationale:* Defines the contract for resolving a chunk ID to its full source details.

- [x] **4.2 Implement brainatlas-be handler for chunk source lookup**
  Add a new method to the `BrainRegionApi` trait (`brainatlas-be/crates/api/src/api.rs`) and implement it in the api layer. The implementation queries `brain_region_embeddings` by `id` (the chunk UUID) and returns all source metadata including s3_key and character offsets.
  *Rationale:* brainatlas-be owns the embeddings data.

- [x] **4.3 Add brainatlas-be Axum route**
  Add `GET /brainatlas-be/api/chunks/{chunk_id}/source` to `brainatlas-be/crates/server/src/server.rs`.
  *Rationale:* New HTTP endpoint for the chunk source resolution.

- [x] **4.4 Add orch proxy endpoint**
  Add `GET /orch/api/chunks/{chunk_id}/source` to `orch/crates/server/src/server.rs:57-76`. The handler forwards the request to `brainatlas-be/api/chunks/{chunk_id}/source`.
  *Rationale:* Orch is the public-facing API gateway; the client should not call brainatlas-be directly.

- [x] **4.5 Add orch services/infra for chunk resolution**
  Add the chunk resolution method to orch's services trait and implement it as an HTTP call to brainatlas-be, following the same pattern as `generate_queries()` in `orch/crates/services/src/region_management.rs:136-190`.
  *Rationale:* Follows the established orch → brainatlas-be HTTP delegation pattern.

### Phase 5: Wire orch's ProcessRegionRequest to include paper_metadata

- [x] **5.1 Add `paper_metadata` field to orch's `ProcessRegionRequest`**
  The proto already defines `repeated PaperMetadata paper_metadata` in `ProcessRegionRequest` (`proto/llm/brain.proto:102`), but orch's local type at `orch/crates/services/src/types.rs:36-46` omits it. Add the field.
  *Rationale:* Without paper_metadata, brainatlas-be cannot associate S3 keys with PMC IDs and other metadata.

- [x] **5.2 Populate paper_metadata in `process_batch()`**
  In `orch/crates/services/src/completion_watcher.rs:226-397`, after collecting S3 keys, also query the database for the corresponding paper metadata (pmc_id, uid, query) and construct `PaperMetadata` entries. Use the `fetch_tasks` and `fetch_task_components` tables to map s3_key → task → pmc_id.
  *Rationale:* The metadata is already in the database from the fetch phase — it just needs to be collected and forwarded.

- [x] **5.3 Add infra method to get paper metadata by task IDs**
  Add `get_task_paper_metadata(database_url, task_ids) -> Vec<PaperMetadata>` to orch's `BatchManagement` trait and implement in `orch/crates/infra/src/pg.rs`. This JOINs `fetch_tasks` with `fetch_task_components` to get `(s3_key, pmc_id, query)` tuples.
  *Rationale:* The data needed for paper_metadata lives across two fetcher tables that orch already has access to.

### Phase 6: Proto and rpc-types synchronization

- [x] **6.1 Regenerate rpc-types after proto changes**
  After modifying `proto/llm/brain.proto` (Phase 4.1) and `proto/orch/orch.proto` (Phase 3.3), rebuild the generated Rust types in `brainatlas-be/crates/rpc-types/` and the orch equivalent. Run `cargo build` in each workspace to trigger `tonic-prost-build`.
  *Rationale:* Proto changes require rebuilding the generated code.

## Verification Criteria

- When `GET /orch/api/regions/{id}/summaries` returns a summary, each `RegionSummary` JSON object includes a `sources` array with chunk IDs, PMC IDs, and UIDs
- The summary text itself contains inline citations (e.g., `[chunk:<uuid>]`) that match entries in the `sources` array
- `GET /orch/api/chunks/{chunk_id}/source` returns the full source details: s3_key, pmc_id, uid, char_start, char_end for a valid chunk UUID
- The `brain_region_embeddings` table stores `source_char_start` and `source_char_end` for newly created embeddings
- Existing summaries without citations continue to work (graceful degradation — `sources` array is empty, no citations in text)

## Potential Risks and Mitigations

1. **LLM may not reliably cite chunk UUIDs**
   Mitigation: Use a distinctive citation format like `[chunk:xxxxxxxx]` that is easy to validate. Add post-processing in `rag_summarize()` to verify cited IDs exist in the chunks that were actually returned by tool calls. Consider Option C (numbered footnotes + backend mapping) as a fallback if UUID citation proves unreliable.

2. **Character offsets may drift if chunker logic changes**
   Mitigation: Store offsets at embedding creation time (not computed on-the-fly). If the chunker parameters change, existing offsets remain valid for their embeddings.

3. **Performance impact of querying sources per summary**
   Mitigation: The query is a simple indexed SELECT on `summary_id` (already indexed via `idx_embeddings_summary`). Can add DISTINCT on source columns to reduce result set size. Consider caching or denormalizing into `region_summary` if needed.

4. **Proto backward compatibility**
   Mitigation: All new proto fields are `optional` or `repeated` (additive changes). Existing clients that don't read the new fields continue to work.

5. **Orch ProcessRegionRequest missing paper_metadata**
   Mitigation: This is a pre-existing gap (Phase 5). The fix is purely additive and the brainatlas-be handler already accepts the field.

## Alternative Approaches

1. **Embed all source metadata directly in the summary text (no separate endpoint):** Trade-off: Makes the summary text bloated and harder to render. Rejected in favor of the two-tier approach (lightweight sources in summary response, detailed source on-demand).

2. **Store a separate `summary_sources` join table instead of deriving from embeddings:** Trade-off: Cleaner data model but adds write complexity. Since embeddings already carry source metadata and are linked to summaries via `summary_id`, the derivation approach avoids redundancy.

3. **Use numbered footnotes with a backend-generated source map instead of LLM-cited UUIDs:** Trade-off: More reliable citation (the backend controls the mapping) but requires post-processing the LLM output to replace numbered references with actual source data. Could be implemented as a future enhancement.

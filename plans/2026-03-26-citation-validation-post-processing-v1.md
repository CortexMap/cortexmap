# Post-Processing Citation Validation for RAG Summarization Pipeline

## Objective

Add a citation validation step to the RAG summarization pipeline in `brainatlas-be`. After the LLM generates a summary with `[chunk:UUID]` citations (returned from `rag_summarize` at `app.rs:266`), validate each citation against its source chunk text using a keyword-overlap heuristic. Remove citations with insufficient relevance before persisting the summary (at `app.rs:200`). This ensures only well-grounded citations survive into the stored summary, improving trust and accuracy.

## Architecture Overview

The change touches **four layers** of the crate hierarchy, following the existing pattern where each new database capability is declared in trait form and plumbed through infra/services:

```
domain   (types only)         — no change needed; reuses existing ChunkSource
app      (business logic)     — new validate_citations method + call site in process_region
app      (services trait)     — new get_chunks_by_ids on VectorDatabase trait
services (service impl)       — wire get_chunks_by_ids through to infra
services (infra trait)        — new get_chunks_by_ids on VectorDatabase infra trait
infra    (vectordb.rs impl)   — new batch query using WHERE id = ANY($1)
infra    (infra.rs facade)    — delegate to vectordb
```

## Implementation Plan

### Layer 1: Infrastructure — Batch Chunk Query (`infra` crate)

- [x] **1.1** In `infra/src/vectordb.rs`, add a new method `get_chunks_by_ids` on the `impl VectorDatabase for BrainAtlasVectorDB` block (after the existing `get_chunk_source` method at line 264). This method should:
  - Accept `database_url: &str` and `chunk_ids: Vec<Uuid>`
  - Return `Result<Vec<ChunkSource>, Self::Error>`
  - Use Diesel's `filter(brain_region_embeddings::id.eq_any(&chunk_ids))` to do a single batch query (no raw SQL needed)
  - Load `Vec<EmbeddingRow>`, then map each to `ChunkSource` using the same mapping pattern as `get_chunk_source` (lines 252-261)
  - Handle the empty `chunk_ids` case gracefully: return `Ok(vec![])` immediately without hitting the database

- [x] **1.2** In `infra/src/infra.rs`, add the delegation method `get_chunks_by_ids` on the `impl VectorDatabase for BrainAtlasInfra` block (after the existing `get_chunk_source` delegation at line 179). Pattern: `self.vectordb.get_chunks_by_ids(database_url, chunk_ids).await`

### Layer 2: Services Infra Trait — Declare the New Capability

- [x] **2.1** In `services/src/infra.rs`, add `get_chunks_by_ids` to the `VectorDatabase` trait (after line 170):
  ```
  async fn get_chunks_by_ids(
      &self,
      database_url: &str,
      chunk_ids: Vec<Uuid>,
  ) -> Result<Vec<ChunkSource>, Self::Error>;
  ```
  **Rationale:** The services-layer infra trait is the contract between the service layer and the infrastructure layer. Every DB operation must be declared here.

### Layer 3: Services Implementation — Wire Through

- [x] **3.1** In `services/src/services.rs`, add `get_chunks_by_ids` to the `impl VectorDatabase for BrainAtlasServices<I>` block (after the existing `get_chunk_source` impl at line 217). Follow the same pattern: resolve `DATABASE_URL` from `self.infra.get("DATABASE_URL")`, then delegate to `self.infra.get_chunks_by_ids(&database_url, chunk_ids)`, mapping errors with `ServiceError::InfraError`.

### Layer 4: App Services Trait — Declare for App Layer

- [x] **4.1** In `app/src/services.rs`, add `get_chunks_by_ids` to the `VectorDatabase` trait (after `get_chunk_source` at line 85):
  ```
  async fn get_chunks_by_ids(
      &self,
      chunk_ids: Vec<Uuid>,
  ) -> Result<Vec<ChunkSource>, Self::Error>;
  ```
  **Note:** The app-layer `VectorDatabase` trait does NOT take `database_url` — that's resolved internally by the services layer. This matches the existing pattern (compare `get_chunk_source` at app layer vs services layer).

### Layer 5: App Business Logic — `validate_citations` Method

- [x] **5.1** Add a new dependency `regex` to `app/Cargo.toml`. This is needed to extract `[chunk:UUID]` patterns from the summary text. Add: `regex = "1"`.

- [x] **5.2** In `app/src/app.rs`, add the import for `regex::Regex` and `std::collections::HashSet` at the top of the file.

- [x] **5.3** In `app/src/app.rs`, implement a new method `validate_citations` on `BrainAtlasApp<S>` (inside the existing `impl` block, after `get_chunk_source` at line 383). The method signature should be:
  ```
  async fn validate_citations(&self, summary_text: &str) -> Result<String, AppError<E>>
  ```

  The method logic should follow these steps:

  **Step A — Extract all `[chunk:UUID]` patterns:**
  - Use a regex pattern: `\[chunk:([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})\]`
  - Collect all unique UUIDs found in the text. Parse each capture group into `Uuid`.
  - If no citations found, log and return the text unchanged.

  **Step B — Batch-fetch chunk texts:**
  - Call `self.services.get_chunks_by_ids(unique_uuids.clone()).await?`
  - Build a `HashMap<Uuid, String>` mapping chunk_id to chunk_text for fast lookup.

  **Step C — Define a stopwords set:**
  - Create a `HashSet<&str>` containing common English stopwords: `the, a, an, is, are, was, were, be, been, being, have, has, had, do, does, did, will, would, shall, should, may, might, can, could, of, in, to, for, with, on, at, by, from, as, into, through, during, before, after, above, below, between, out, off, over, under, again, further, then, once, and, but, or, nor, not, so, yet, both, either, neither, each, every, all, any, few, more, most, other, some, such, no, only, own, same, than, too, very, just, about, also, its, it, this, that, these, those, which, who, whom, what, where, when, how, their, they, them, he, she, his, her, we, our, you, your`.
  - This is intentionally a generous list to ensure only substantive keyword matches count.

  **Step D — For each citation occurrence, validate:**
  - Iterate through all regex matches (not just unique — process each occurrence in context).
  - For each match, extract the "claim sentence": the text from the previous sentence boundary (`.`, `!`, `?`, or start of text) up to the citation marker. Use a simple heuristic: search backwards from the match start position for the nearest sentence-ending punctuation, then take the text between that punctuation and the citation.
  - Tokenize the claim sentence into lowercase words (split on non-alphanumeric characters, filter out tokens shorter than 3 characters).
  - Filter out stopwords.
  - Look up the chunk text from the HashMap. If the chunk UUID is not found in the DB results (orphaned citation), mark it for removal.
  - Tokenize the chunk text the same way (lowercase, split, filter stopwords, min length 3).
  - Compute intersection: count how many non-trivial words from the claim sentence appear in the chunk text's word set.
  - **Threshold:** If fewer than 2 non-trivial shared words, mark the citation for removal.

  **Step E — Remove invalid citations and log:**
  - Build the output string by replacing each invalid `[chunk:UUID]` with an empty string.
  - Log at `info!` level: "Citation validation: {validated} validated, {removed} removed out of {total} total citations"
  - If any citations were removed, also log at `warn!` level each removed citation UUID for debugging.
  - Return the cleaned summary text.

- [x] **5.4** In `app/src/app.rs`, in the `process_region` method, insert the validation call between the `rag_summarize` call (line 190-197) and the `update_summary_text` call (line 200-203). The modified flow should be:
  ```
  // 7. RAG summarization loop
  let summary_text = self.rag_summarize(...).await?;

  // 7.5 Validate citations
  let summary_text = self.validate_citations(&summary_text).await?;

  // 8. Update the summary record with the final text
  self.services.update_summary_text(summary_id, &summary_text).await...
  ```
  **Rationale:** The validation must happen AFTER the LLM generates the summary but BEFORE persisting it, so only validated citations are stored in the database.

### Layer 6: Workspace Cargo.toml

- [x] **6.1** Check the workspace `Cargo.toml` at `brainatlas-be/Cargo.toml`. If `regex` is not already declared in `[workspace.dependencies]`, add `regex = "1"` there and reference it as `regex.workspace = true` in `app/Cargo.toml`. If the workspace doesn't use workspace-level dependency management for this kind of dependency, add `regex = "1"` directly to `app/Cargo.toml`.

## Verification Criteria

- The project compiles cleanly with `cargo check` from `brainatlas-be/`.
- `get_chunks_by_ids` is available at all four layers (infra trait, infra impl, services trait, services impl, app trait) and returns the correct `Vec<ChunkSource>` for a set of UUIDs.
- `validate_citations` correctly extracts `[chunk:UUID]` patterns from sample text.
- Citations with fewer than 2 non-trivial shared words between the claim sentence and chunk text are removed from the output.
- Citations with sufficient keyword overlap are preserved unchanged.
- Orphaned citations (UUID not found in database) are removed.
- Empty input (no citations) passes through unchanged.
- The info log line shows validated vs removed counts.
- The `process_region` pipeline calls `validate_citations` before `update_summary_text`.

## Potential Risks and Mitigations

1. **Regex compilation cost on every call**
   Mitigation: Use `std::sync::LazyLock` (stable since Rust 1.80) or `once_cell::sync::Lazy` to compile the regex once as a static. Since the `app` crate targets edition 2024 (see `Cargo.toml:4`), `LazyLock` is available.

2. **Sentence boundary detection is imperfect**
   Mitigation: The heuristic (scan backwards for `.`, `!`, `?`) is intentionally simple. It may occasionally grab too much or too little context, but this is acceptable for a keyword-overlap check. Abbreviations like "e.g." could cause false splits, but the overlap threshold of 2 words is forgiving enough to tolerate minor boundary errors.

3. **Stopword list may be incomplete for scientific text**
   Mitigation: The list covers general English stopwords. Domain-specific filler words (e.g., "study", "results", "found") are NOT in the stopword list, which is correct — these words carry signal in a neuroscience context and indicate the claim is about the research findings.

4. **Large number of citations causing many DB lookups**
   Mitigation: The batch query (`WHERE id = ANY($1)`) fetches all chunk texts in a single round-trip. Even summaries with 50+ citations will require only one DB query.

5. **Threshold too aggressive (removes valid citations)**
   Mitigation: A threshold of 2 non-trivial shared words is intentionally conservative. The LLM is prompted to write claims based on chunk content, so genuine citations will almost always share multiple substantive terms. If needed, the threshold can be tuned down to 1 in the future.

## Alternative Approaches

1. **Use TF-IDF or cosine similarity on word vectors instead of keyword overlap**: More sophisticated but adds complexity (needs a vocabulary, IDF weights). Overkill for a first pass — keyword overlap is interpretable and fast.

2. **Use a second LLM call to validate citations**: Higher accuracy but adds latency, cost, and another failure point. Explicitly ruled out by requirements.

3. **Validate during RAG loop (as citations are generated) instead of post-processing**: Would require parsing partial LLM output mid-stream and re-prompting, which is significantly more complex. Post-processing is simpler and catches all citations uniformly.

4. **Skip the batch query and use the existing single `get_chunk_source` in a loop**: Simpler implementation (no new trait method needed) but causes N sequential database round-trips. The batch query is worth the plumbing effort for performance.

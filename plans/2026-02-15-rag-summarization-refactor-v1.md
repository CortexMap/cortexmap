# RAG-Based Summary Generation Refactor

## Objective

Replace the current "dump all chunks into user prompt" summarization approach with a proper RAG (Retrieval-Augmented Generation) pipeline using tool calling. Currently, `process_region` in `brainatlas-be/crates/app/src/app.rs:41-128` chunks papers, generates embeddings, and then naively passes **all chunks as a concatenated user message** to the LLM for summarization. Instead, the flow should:

1. **Insert chunks + embeddings into the vector DB first**
2. **Call the LLM with a tool definition** (`search_embeddings`) so the model can request relevant context
3. **Execute the vector similarity search** when the LLM makes a tool call
4. **Return the search results** to the LLM so it can synthesize a summary from retrieved context

This is a proper agentic RAG loop: LLM decides what to search for, we execute, LLM synthesizes.

---

## Current Flow (Broken)

```
app.rs process_region:
  1. Download S3 files → concatenate full_text
  2. Compute content hash for dedup
  3. Chunk text (1000 chars, 200 overlap)
  4. Generate embeddings for all chunks in parallel
  5. Build NewEmbedding structs
  6. ❌ Summarize: pass ALL chunks as user prompt (no retrieval, just concatenation)
  7. Insert summary + embeddings atomically
```

**Problem**: Step 6 feeds raw chunks directly into the LLM context window. This doesn't scale (context limit), doesn't let the LLM focus on relevant information, and wastes the embeddings that were just generated.

---

## Target Flow

```
app.rs process_region:
  1. Download S3 files → concatenate full_text
  2. Compute content hash for dedup
  3. Chunk text (1000 chars, 200 overlap)
  4. Generate embeddings for all chunks in parallel
  5. Build NewEmbedding structs
  6. Insert summary placeholder + embeddings atomically (embeddings now searchable)
  7. ✅ RAG summarization loop:
     a. Call LLM with system prompt + tool definition (search_embeddings)
     b. LLM makes tool call with a query string
     c. Execute vector similarity search against the just-inserted embeddings
     d. Return search results as tool response
     e. LLM synthesizes final summary from retrieved context
     f. Repeat b-e if LLM makes additional tool calls
  8. Update the summary record with the final LLM-generated text
```

---

## Implementation Plan

### Layer 1: Domain — New types for tool calling and similarity search

- [x] **1.1** Add `SimilarChunk` struct to `brainatlas-be/crates/domain/src/processing.rs` representing a chunk returned from vector search:
  ```
  struct SimilarChunk {
      chunk_index: i32,
      chunk_text: String,
      similarity_score: f64,
  }
  ```

- [x] **1.2** Add `ToolCall` and `ToolResult` domain types to a new file `brainatlas-be/crates/domain/src/tool_calling.rs`:
  ```
  struct ToolCall {
      id: String,
      name: String,       // "search_embeddings"
      arguments: String,   // JSON string: {"query": "...", "top_k": 5}
  }

  struct ToolResult {
      tool_call_id: String,
      content: String,     // JSON-serialized Vec<SimilarChunk>
  }

  enum LlmResponse {
      ToolCalls(Vec<ToolCall>),
      Final(String),
  }
  ```

- [x] **1.3** Expose the new module from `brainatlas-be/crates/domain/src/lib.rs`

### Layer 2: Infra — Vector similarity search + tool-calling LLM client

- [x] **2.1** Add `search_similar` method to the `VectorDatabase` infra trait in `brainatlas-be/crates/services/src/infra.rs`:
  ```
  async fn search_similar(
      &self,
      database_url: &str,
      query_embedding: Vec<f32>,
      region_id: i32,
      top_k: usize,
  ) -> Result<Vec<SimilarChunk>, Self::Error>;
  ```
  **Rationale**: The infra layer needs a way to perform cosine similarity search against `brain_region_embeddings` using pgvector's `<=>` operator, scoped to a specific region.

- [x] **2.2** Implement `search_similar` in `brainatlas-be/crates/infra/src/vectordb.rs` using Diesel raw SQL or `diesel::sql_query` with pgvector's cosine distance operator (`<=>`):
  ```sql
  SELECT chunk_index, chunk_text, 1 - (embedding <=> $1::vector) AS similarity_score
  FROM brain_region_embeddings
  WHERE region_id = $2
  ORDER BY embedding <=> $1::vector
  LIMIT $3
  ```

- [x] **2.3** Replace `LlmClient::summarize` trait method in `brainatlas-be/crates/services/src/infra.rs` with a new `summarize_with_tools` method:
  ```
  async fn summarize_with_tools(
      &self,
      api_key: &str,
      chat_model: &str,
      region_name: &str,
      messages: Vec<(String, String)>,  // (role, content) pairs
      tools: &[serde_json::Value],
  ) -> Result<LlmResponse, Self::Error>;
  ```
  **Rationale**: The old `summarize` just accepted chunks. The new method accepts a message history (for multi-turn tool-call conversations) and tool definitions. It returns either `LlmResponse::ToolCalls` or `LlmResponse::Final`.

- [x] **2.4** Update the `Infra` blanket trait bound in `brainatlas-be/crates/services/src/infra.rs` to include the updated `LlmClient` shape.

- [x] **2.5** Implement `summarize_with_tools` in `brainatlas-be/crates/infra/src/llm.rs` on `OpenRouterClient`:
  - Add `tools` field to `ChatRequest` as `Option<Vec<serde_json::Value>>`
  - Add `tool_calls` field to the `ChatMessage` response deserializer
  - Add `ToolCallResponse` deserialization struct with `id`, `function.name`, `function.arguments`
  - Parse the response: if `tool_calls` is present → return `LlmResponse::ToolCalls`; if `content` is present → return `LlmResponse::Final`
  - OpenRouter/OpenAI tool calling format reference:
    ```json
    {
      "tools": [{
        "type": "function",
        "function": {
          "name": "search_embeddings",
          "description": "Search the vector database for chunks relevant to a query about this brain region",
          "parameters": {
            "type": "object",
            "properties": {
              "query": { "type": "string", "description": "Natural language search query" },
              "top_k": { "type": "integer", "description": "Number of results to return", "default": 5 }
            },
            "required": ["query"]
          }
        }
      }]
    }
    ```

- [x] **2.6** Remove the old `summarize` method from both the trait and `OpenRouterClient` impl. Update `BrainAtlasInfra` delegation in `brainatlas-be/crates/infra/src/infra.rs`.

- [x] **2.7** Add a new prompt template `brainatlas-be/crates/infra/prompts/summarize_rag_system.md`:
  ```
  You are a neuroscience expert. You have access to a search tool that can find relevant passages from research papers about a specific brain region.

  Use the search_embeddings tool to find relevant information, then synthesize a comprehensive summary. You may call the tool multiple times with different queries to gather information about:
  1. Key anatomical features and connectivity
  2. Primary functions and role in cognition/behavior
  3. Clinical significance and disorders
  4. Recent research findings

  Be comprehensive but concise. Use scientific terminology appropriately.
  When you have gathered enough information, provide your final summary directly (without a tool call).
  ```

### Layer 3: Services — Orchestrate the RAG loop

- [x] **3.1** Add `search_similar` to the `VectorDatabase` app-level trait in `brainatlas-be/crates/app/src/services.rs`:
  ```
  async fn search_similar(
      &self,
      query_embedding: Vec<f32>,
      region_id: i32,
      top_k: usize,
  ) -> Result<Vec<SimilarChunk>, Self::Error>;
  ```

- [x] **3.2** Implement `search_similar` delegation in `BrainAtlasServices` (`brainatlas-be/crates/services/src/services.rs`) — reads `DATABASE_URL` from env and delegates to infra.

- [x] **3.3** Update `BrainAtlasLlmService` in `brainatlas-be/crates/services/src/llm_service.rs`:
  - Replace `summarize` with `summarize_with_tools` that reads `OPENROUTER_API_KEY` and `CHAT_MODEL` then delegates to infra.

- [x] **3.4** Update the `LlmService` app-level trait in `brainatlas-be/crates/app/src/services.rs`:
  - Replace `summarize(chunks)` with `summarize_with_tools(region_name, messages, tools)` matching the new infra trait shape.

### Layer 4: App — Rewrite the `process_region` flow

- [x] **4.1** Rewrite `process_region` in `brainatlas-be/crates/app/src/app.rs` with the new RAG loop:
  ```
  1. Download S3 files → full_text
  2. Compute content hash, dedup check
  3. Chunk text
  4. Generate embeddings in parallel
  5. Build NewEmbedding structs
  6. Insert placeholder summary + embeddings atomically
     (summary text = "" or "Generating...", embeddings are now searchable)
  7. RAG summarization loop (max_iterations = 5):
     a. Build tool definition for search_embeddings
     b. Call summarize_with_tools with system prompt + region_name
     c. Match on LlmResponse:
        - ToolCalls → for each call:
          i.  Parse arguments (query string, top_k)
          ii. Generate embedding for the query string
          iii. Call search_similar with that embedding + region_id
          iv. Format results as tool response message
          v.  Add tool_call + tool_result to message history
        - Final(text) → break loop, use text as summary
  8. Update the summary record with final text
  ```

- [x] **4.2** Add an `update_summary` method to the `VectorDatabase` app-level trait and implement it through the layers:
  - App trait in `brainatlas-be/crates/app/src/services.rs`
  - Services impl in `brainatlas-be/crates/services/src/services.rs`
  - Infra trait in `brainatlas-be/crates/services/src/infra.rs`
  - Infra impl in `brainatlas-be/crates/infra/src/vectordb.rs`
  ```sql
  UPDATE region_summary SET summary = $1 WHERE id = $2
  ```

- [x] **4.3** Add `MaxToolCallsExceeded` variant to `AppError` in `brainatlas-be/crates/app/src/error.rs` for safety if the LLM loops too many times.

### Layer 5: Cleanup

- [x] **5.1** Remove the old `summarize_user.md` and `summarize_system.md` prompt templates (replaced by `summarize_rag_system.md`). Remove the `load_prompt` entries for them.

- [x] **5.2** Update the `load_prompt` function in `brainatlas-be/crates/infra/src/llm.rs` to include the new prompt template.

---

## Verification Criteria

- The `brain_region_embeddings` rows are inserted **before** the LLM is called for summarization
- The LLM receives a `tools` array with the `search_embeddings` function definition
- When the LLM returns `tool_calls`, the system generates an embedding for the query, executes vector similarity search, and returns results
- The conversation continues (multi-turn) until the LLM returns a final text response
- The summary record is updated with the final generated text
- A safety limit prevents infinite tool-call loops (e.g., max 5 iterations)
- `cargo check --workspace` passes in `brainatlas-be/`

---

## Potential Risks and Mitigations

1. **Model doesn't support tool calling**
   Mitigation: The configured `chat_model` must support OpenAI-compatible tool calling (e.g., `openai/gpt-4o-mini`). Document this requirement. The `summarize_with_tools` implementation should gracefully fall back if the model returns content instead of tool_calls on the first turn.

2. **Infinite tool-call loop**
   Mitigation: Hard cap at 5 iterations in the RAG loop. After max iterations, use whatever summary the model has produced so far or return an error.

3. **Empty vector search results**
   Mitigation: If `search_similar` returns no results for a query, still send an empty results array back to the LLM so it can adjust its query or produce a summary from what it already has.

4. **Placeholder summary visible to users**
   Mitigation: Insert summary with a "generating" marker. The orch completion watcher already tracks batch processing status, so consumers know the summary isn't ready until processing completes.

5. **Embeddings inserted but summary generation fails**
   Mitigation: The embeddings are still valid and useful. The summary can be retried. Consider adding a `summary_status` field or rely on the existing batch status tracking.

---

## Alternative Approaches

1. **Simple RAG without tool calling**: Skip tool calling entirely. After inserting embeddings, generate a query embedding from the region name, do a single similarity search, and pass the top-K results as context to the LLM. Simpler but less flexible — the LLM can't iteratively refine its search.

2. **Streaming multi-query RAG**: Generate N predefined queries (anatomy, function, disease, etc.) upfront, run similarity search for each, merge results, and pass the combined context to the LLM in a single call. Deterministic, no tool calling needed, but less adaptive.

3. **Hybrid approach**: First call generates tool calls, but cap at 1-2 tool call rounds before forcing a final response. Balances flexibility with predictability.

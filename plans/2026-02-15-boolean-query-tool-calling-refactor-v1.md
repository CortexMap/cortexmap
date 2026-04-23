# Boolean Query Tool-Calling Refactor

## Objective

Fix the query generation bug where the LLM produces raw text queries like `"Interpeduncular fossa function" AND "neurotransmitters"` that get passed verbatim to PubMed's ESearch API, returning 0 results. Instead, leverage LLM tool-calling to produce structured `BooleanQuery` JSON that can be properly formatted for PubMed using the existing `BooleanQuery::to_string()` method (which correctly handles URL encoding, quoting, parenthesization, and boolean operators).

## Problem Analysis

### Current Flow (Broken)
```
Orch → brainatlas /generate-queries 
     → LLM generates plain text: "Interpeduncular fossa function" AND "neurotransmitters" AND ...
     → Orch passes raw string to fetcher /enqueue
     → Fetcher sets blueprint.fetcher.query = raw string
     → enqueue.rs substitutes into esearch_url: &term=<raw string>
     → PubMed treats entire string as literal, returns 0 results
```

### Root Cause
- `brainatlas-be/crates/infra/src/llm.rs:332-353` — `generate_queries` parses the LLM response as one-query-per-line raw text
- `orch/crates/services/src/batch_orchestration.rs:120-123` — passes this raw string to fetcher `EnqueueRequest.query`
- `fetcher-be/crates/cortexmap-be/src/server.rs:139` — `blueprint.fetcher.query = req.query.clone()` uses it as-is
- `fetcher-be/crates/cortexmap-fetcher/src/enqueue.rs:20-22` — substitutes into PubMed URL without any formatting

### Target Flow
```
Orch → brainatlas /generate-queries
     → LLM gets a tool definition for "create_pubmed_query"
     → LLM makes tool call with BooleanQuery JSON: { "and": [...] }
     → brainatlas parses into BooleanQuery, calls .to_string()
     → Returns properly formatted string: "((\"motor+cortex\"+OR+M1)+AND+(fMRI+OR+optogenetics))"
     → Orch passes formatted string to fetcher /enqueue
     → Fetcher uses it → PubMed returns results
```

## Key Architectural Decision

The `BooleanQuery` type lives in `fetcher-be/crates/cortexmap-core` (separate workspace from `brainatlas-be`). Rather than creating a cross-workspace dependency, **copy the `BooleanQuery` type and its serde serialization into brainatlas-be's domain crate** as a "query builder" module. This is justified because:
- The type is relatively small (~100 lines of enum + serde derives + builder methods + Display)
- The two workspaces may evolve independently
- We only need the type for parsing LLM tool-call output and generating the query string

## Implementation Plan

### Layer 1: Domain — Add BooleanQuery to brainatlas-be

- [x] **1.1** Create `brainatlas-be/crates/domain/src/boolean_query.rs` — Copy the `BooleanQuery` enum, `FieldQuery`, `NotQuery`, `BoostQuery`, `RangeQuery` structs from `fetcher-be/crates/cortexmap-core/src/config/query.rs:1-243`. Include the `#[derive(Serialize, Deserialize)]` attributes and the `to_string_inner()` / `to_string()` methods. Also add serde_json deserialization support (it already uses `#[serde(rename_all = "lowercase")]`). **Rationale**: This gives brainatlas-be the ability to parse LLM tool-call JSON into a `BooleanQuery` and format it for PubMed.

- [x] **1.2** Expose the module from `brainatlas-be/crates/domain/src/lib.rs` — Add `pub mod boolean_query;` and re-export `BooleanQuery` and its sub-types. **Rationale**: Make the type available to the infra and app layers.

### Layer 2: Infra — Update generate_queries to use tool calling

- [x] **2.1** Create a new prompt file `brainatlas-be/crates/infra/prompts/generate_queries_tool_system.md` — This prompt instructs the LLM:
  - It is a neuroscience research librarian
  - It MUST use the `create_pubmed_query` tool to generate each query
  - It should generate queries using `BooleanQuery` JSON format
  - Explain the `BooleanQuery` schema in the system prompt: `term`, `phrase`, `and`, `or`, `not`, `field`, `boost`, `range` variants
  - Show an example: for "motor cortex", produce `{"and": [{"or": [{"term": "motor cortex"}, {"term": "M1"}]}, {"or": [{"term": "fMRI"}, {"term": "optogenetics"}]}]}`
  - Emphasize: generate `{count}` distinct queries, each as a separate tool call
  **Rationale**: The LLM needs clear instructions and the JSON schema to produce valid `BooleanQuery` structures.

- [x] **2.2** Update `load_prompt()` in `brainatlas-be/crates/infra/src/llm.rs:10-17` — Add `"generate_queries_tool_system"` case to load the new prompt. **Rationale**: Make the new prompt available at compile time.

- [x] **2.3** Rewrite `LlmClient::generate_queries` in `brainatlas-be/crates/services/src/infra.rs` — No change needed; trait signature stays `async fn generate_queries(...) -> Result<Vec<String>, ...>` as planned. **Rationale**: Keep the external interface stable; only the internal implementation changes.

- [x] **2.4** Rewrite `OpenRouterClient::generate_queries` in `brainatlas-be/crates/infra/src/llm.rs:260-545` — Instead of a simple chat request that returns text, use tool-calling:
  1. Build a `ChatRequest` with the new system prompt and a `tools` array containing:
     ```json
     {
       "type": "function",
       "function": {
         "name": "create_pubmed_query",
         "description": "Create a structured PubMed search query for finding academic papers",
         "parameters": {
           "type": "object",
           "properties": {
             "query": {
               "description": "A BooleanQuery JSON object representing the search",
               "$ref": "#/$defs/BooleanQuery"
             }
           },
           "required": ["query"],
           "$defs": {
             "BooleanQuery": {
               "oneOf": [
                 { "type": "object", "properties": { "term": { "type": "string" } }, "required": ["term"] },
                 { "type": "object", "properties": { "phrase": { "type": "string" } }, "required": ["phrase"] },
                 { "type": "object", "properties": { "and": { "type": "array", "items": { "$ref": "#/$defs/BooleanQuery" } } }, "required": ["and"] },
                 { "type": "object", "properties": { "or": { "type": "array", "items": { "$ref": "#/$defs/BooleanQuery" } } }, "required": ["or"] },
                 { "type": "object", "properties": { "field": { "type": "object", "properties": { "name": {"type":"string"}, "value": {"type":"string"} }, "required": ["name","value"] } }, "required": ["field"] }
               ]
             }
           }
         }
       }
     }
     ```
  2. Send the request, get tool call responses
  3. For each tool call, parse the `arguments` JSON as `BooleanQuery`
  4. Call `boolean_query.to_string()` to get the properly formatted PubMed query string
  5. If the LLM returns text instead of tool calls (fallback), log a warning and attempt to parse the text as JSON, or fall back to the old per-line behavior with a deprecation warning
  6. Run a multi-turn loop (similar to `rag_summarize`):
     - After each tool call, respond with `"role": "tool", "content": "Query created successfully: <formatted_string>"`
     - Continue until the LLM either returns all `count` queries via tool calls, or provides a final text response
     - Max 3 iterations to prevent infinite loops
  **Rationale**: This is the core fix. The LLM produces structured JSON, we parse it into `BooleanQuery`, and `to_string()` produces a properly formatted PubMed query string.

### Layer 3: Service — Wire through the changes

- [x] **3.1** No changes needed to `BrainAtlasLlmService` or `BrainAtlasEmbeddingService` — The `generate_queries` method signature stays `(api_key, chat_model, region_name, count) -> Vec<String>`. The `BooleanQuery` parsing happens inside the infra layer and returns formatted strings. **Rationale**: The service layer just delegates; the structured query logic is an infra concern.

- [x] **3.2** Verify the `LlmService` app-level trait (`brainatlas-be/crates/app/src/services.rs`) doesn't need changes — It exposes `generate_queries(region_name, count) -> Vec<String>`. This stays the same. **Rationale**: Formatted strings flow out unchanged.

### Layer 4: No changes to orch or fetcher

- [x] **4.1** Verify orch `batch_orchestration.rs:120-123` still works — The `EnqueueRequest.query` field receives a `String`. Previously it was raw text; now it's a properly formatted PubMed query string from `BooleanQuery::to_string()`. The fetcher's `enqueue_query` function at `fetcher-be/crates/cortexmap-fetcher/src/enqueue.rs:20-22` does `blueprint.fetcher.esearch_url.replace("{query}", &blueprint.fetcher.query)` — this will now substitute the formatted string, which is exactly what PubMed expects. **Rationale**: The formatted string is already URL-ready (BooleanQuery::to_string() replaces spaces with `+`).

### Layer 5: Prompt cleanup

- [x] **5.1** Old prompts `generate_queries_system.md` and `generate_queries_user.md` kept as fallback — They may still be useful as a fallback. For now, keep them but update the `load_prompt` function to include the new prompt key. **Rationale**: The old prompts can serve as a text-based fallback if tool calling fails or isn't supported by the model.

## Verification Criteria

- When `generate_queries("hippocampus", 3)` is called, the returned strings should look like properly formatted PubMed queries, e.g.:
  - `("hippocampus"+AND+("anatomy"+OR+"connectivity"))`
  - `("hippocampal+formation"+AND+"neurogenesis")`
  - Not: `Hippocampus anatomy AND connectivity`
- The fetcher should receive these formatted strings and successfully get non-zero results from PubMed's ESearch API
- If the LLM doesn't support tool calling (text fallback), queries should still be generated (with a warning log)
- All workspaces compile cleanly: `cargo check --workspace` in both `brainatlas-be/` and `fetcher-be/`

## Potential Risks and Mitigations

1. **LLM produces invalid BooleanQuery JSON**
   Mitigation: Wrap the `serde_json::from_str` parse in error handling. If a tool call's arguments can't be parsed as `BooleanQuery`, log a warning and try to extract the query as a plain term. Fall back to `BooleanQuery::term(raw_text)` wrapping.

2. **LLM returns text instead of tool calls (model doesn't support tools)**
   Mitigation: Detect `LlmResponse::Final(text)` in the loop and fall back to the old line-by-line parsing behavior. Log a warning recommending a model that supports tool calling.

3. **LLM generates fewer tool calls than requested count**
   Mitigation: After the loop, if fewer than `count` queries were collected, the existing behavior already handles this by returning however many were generated. Orch at `orch/crates/app/src/app.rs:226-234` already checks for empty queries.

4. **`BooleanQuery` copy diverges from fetcher-be's version**
   Mitigation: The copy is a snapshot of a stable, well-tested enum. If the fetcher version evolves, a simple diff will identify changes. Consider extracting to a shared crate in the future.

5. **PubMed query format edge cases**
   Mitigation: `BooleanQuery::to_string()` is already well-tested (see `fetcher-be/crates/cortexmap-core/src/config/query.rs:378-465`). The existing tests cover terms, phrases, AND/OR/NOT, fields, boosts, ranges, and complex nested queries.

## Alternative Approaches

1. **JSON schema validation without tool calling**: Instead of tool calling, instruct the LLM to output JSON directly and parse it. Downside: less reliable — LLMs sometimes wrap JSON in markdown code blocks or add preamble text. Tool calling has a structured output contract.

2. **Shared crate for BooleanQuery**: Extract `BooleanQuery` into a standalone `query-types` crate shared between fetcher-be and brainatlas-be workspaces. Upside: single source of truth. Downside: adds cross-workspace dependency management complexity and slows down independent development.

3. **Modify fetcher to accept BooleanQuery JSON**: Instead of passing formatted strings, pass the raw `BooleanQuery` JSON to fetcher, and let fetcher parse and format. Downside: requires changing the fetcher's enqueue API, proto definitions, and orch's request types.

## Post-Implementation Improvement

**Using `schemars` for automatic JSON schema generation**: Instead of hardcoding the JSON schema in the tool definition, we use the `schemars` crate to auto-generate it from the `BooleanQuery` type. This ensures the schema always stays in sync with the type definition.

Changes:
- Added `schemars = "0.8"` to workspace dependencies (`brainatlas-be/Cargo.toml:33`)
- Added `#[derive(JsonSchema)]` to `BooleanQuery` and all sub-types (`boolean_query.rs:4,37,51,58,68`)
- Updated `llm.rs:4,286-290` to use `schema_for!(BooleanQuery)` and serialize it to JSON for the tool definition

Benefits:
- Schema stays in sync with type automatically
- Reduced code duplication (~85 lines of hardcoded JSON schema removed)
- Type-safe: schema changes when type changes, compile-time guarantee

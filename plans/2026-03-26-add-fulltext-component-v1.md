# Add Full-Text Extraction from PMC XML as a New Fetcher Component

## Objective

Add a `FullText` component type to the fetcher pipeline that extracts the complete article body text from PMC XML (the `<body>` section returned by the existing `EFETCH_URL`) and stores it as `fulltext.txt` in S3. This gives the downstream RAG pipeline (brainatlas) actual paper content to cite against, rather than just abstracts and summaries.

## Architecture Overview

The change touches **7 files across 4 crates** plus a **database migration** and a **protobuf definition**. The existing `EFETCH_URL` already returns full article XML containing a `<body>` section; we reuse it exactly as `fetch_abstract` does, but extract `<body>` instead of `<abstract>`.

**Data flow:**
```
enqueue_task() creates 4 component rows (summary, abstract, pdf, fulltext)
    → worker picks up task → processes pending components
    → fetch_component() matches ComponentType::FullText
    → calls fetch_fulltext() using EFETCH_URL
    → extracts <body> XML → strips tags → returns ComponentResult::FullText(String)
    → uploads to S3 at {prefix}/{PMCID}/fulltext.txt
    → orch completion_watcher filters out .pdf → fulltext.txt passes through ✓
```

## Implementation Plan

### Layer 1: Database Migration (prerequisite — must run before code changes)

- [x] **1.1 Create SQL migration: `ALTER TABLE` to expand the `component_type` CHECK constraint**
  - File: `fetcher-be/migrations/2026-03-26-000001-add_fulltext_component_type/up.sql`
  - The existing constraint at `fetcher-be/migrations/2026-01-29-212151-0000_create_fetch_task_components/up.sql:5` is:
    ```sql
    CHECK (component_type IN ('summary', 'abstract', 'pdf'))
    ```
  - The migration must drop the old CHECK and add a new one:
    ```sql
    ALTER TABLE fetch_task_components
      DROP CONSTRAINT fetch_task_components_component_type_check;
    ALTER TABLE fetch_task_components
      ADD CONSTRAINT fetch_task_components_component_type_check
      CHECK (component_type IN ('summary', 'abstract', 'pdf', 'fulltext'));
    ```
  - **Rationale:** Without this, inserting a `fulltext` component row will fail with a Postgres constraint violation.

- [x] **1.2 Create the corresponding `down.sql`**
  - File: `fetcher-be/migrations/2026-03-26-000001-add_fulltext_component_type/down.sql`
  - Reverses the constraint change back to the original 3 values. Should also `DELETE FROM fetch_task_components WHERE component_type = 'fulltext'` to clean up any rows that would violate the restored constraint.

### Layer 2: Infrastructure Enum (`cortexmap-infra`)

- [x] **2.1 Add `FullText` variant to `ComponentType`**
  - File: `fetcher-be/crates/cortexmap-infra/src/infra.rs:70-74`
  - Add `FullText` to the enum (after `Pdf`). The `strum(serialize_all = "lowercase")` attribute will auto-serialize it as `"fulltext"`.
  - Update the `as_str()` match arm at `infra.rs:77-84` to add:
    ```
    ComponentType::FullText => "fulltext",
    ```

- [x] **2.2 Add fulltext fields to `ComponentStats`**
  - File: `fetcher-be/crates/cortexmap-infra/src/infra.rs:250-258`
  - Add two new fields: `fulltext_completed: i64` and `fulltext_failed: i64`.
  - **Rationale:** The `/api/queue/status` endpoint serializes `ComponentStats` into the `ComponentStatistics` proto message. The new fields allow monitoring of fulltext fetch success rates.

### Layer 3: Fetch Function (`cortexmap-fetcher`)

- [x] **3.1 Create `fetch_fulltext` function in a new module `fetch/fulltext.rs`**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/fetch/fulltext.rs` (new file)
  - The function signature should mirror `fetch_abstract`:
    ```rust
    pub async fn fetch_fulltext<I: HttpInfra>(
        pmc_uid: &str,
        ctx: InfraContext<I>,
    ) -> Result<String, FetchError>
    ```
  - **Logic:**
    1. Import `EFETCH_URL` from `metadata.rs` — this requires making `EFETCH_URL` `pub(crate)` (currently `const`, private). Alternatively, define a local copy or a shared helper. The cleanest approach: change `EFETCH_URL` visibility at `metadata.rs:10` to `pub(crate)`.
    2. Strip "PMC" prefix using the existing `strip_pmc_prefix` helper (also needs `pub(crate)` visibility, currently private at `metadata.rs:14`).
    3. Call `ctx.infra.get(&fetch_url).await` to get XML.
    4. Extract the `<body>` section using the same tag-stripping approach as `extract_abstract_from_xml` but targeting `<body>` instead of `<abstract>`.
  - **The `extract_body_from_xml` helper** should:
    1. Find `<body` tag (may have attributes like `<body id="...">`)
    2. Extract content between `<body...>` and `</body>`
    3. Preserve section structure: convert `<sec>` / `<title>` / `<p>` tags to readable plain text with section headers (similar to the abstract extractor at `metadata.rs:217-248`)
    4. Additionally handle body-specific XML tags: `<fig>`, `<table-wrap>`, `<xref>`, `<ext-link>`, `<sup>`, `<sub>`, `<italic>`, `<bold>`, `<list>`, `<list-item>`, `<label>`, `<caption>`
    5. Strip remaining XML/HTML tags and decode HTML entities (`&amp;`, `&lt;`, `&gt;`, `&#8239;`, `&#x202f;`)
    6. Return `FetchError::NotFound` if no `<body>` section exists (not all PMC articles have full text)
  - **Include unit tests** for `extract_body_from_xml` covering:
    - Successful extraction with sections, paragraphs, figures, tables
    - Handling of nested tags and inline formatting
    - Missing `<body>` section returns `NotFound`
    - HTML entity decoding

- [x] **3.2 Make `EFETCH_URL` and `strip_pmc_prefix` visible to sibling modules**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/fetch/metadata.rs:10,14`
  - Change `const EFETCH_URL` to `pub(crate) const EFETCH_URL` at line 10.
  - Change `fn strip_pmc_prefix` to `pub(crate) fn strip_pmc_prefix` at line 14.
  - **Rationale:** `fulltext.rs` is a sibling module in the same `fetch/` directory and needs access to these items. Using `pub(crate)` keeps them internal to the crate.

- [x] **3.3 Register the new module in `fetch/mod.rs`**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/fetch/mod.rs`
  - Add `pub mod fulltext;` alongside the existing `metadata` and `pdf` modules.

- [x] **3.4 Export `fetch_fulltext` from the crate root**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/lib.rs:12`
  - Add `fetch_fulltext` to the public exports:
    ```rust
    pub use fetch::fulltext::fetch_fulltext;
    ```

### Layer 4: Component Integration (`cortexmap-fetcher`)

- [x] **4.1 Add `ComponentResult::FullText` variant**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/component.rs:10-14`
  - Add a new variant: `FullText(String)` (same shape as `Abstract(String)`)

- [x] **4.2 Implement `key_suffix()` for `FullText`**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/component.rs:18-24`
  - Add match arm: `ComponentResult::FullText(_) => "fulltext.txt"`

- [x] **4.3 Implement `into_byte_stream()` for `FullText`**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/component.rs:27-48`
  - Add match arm identical to the `Abstract` case — wrap the text string as a single-element byte stream.

- [x] **4.4 Add `FullText` arm to `fetch_component()`**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/component.rs:124-143`
  - Add:
    ```rust
    ComponentType::FullText => {
        let fulltext = crate::fetch::fulltext::fetch_fulltext(&pmc_id, ctx).await?;
        Ok(ComponentResult::FullText(fulltext))
    }
    ```

- [x] **4.5 Add `FullText` arm to `determine_component_key()`**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/component.rs:146-158`
  - Add match arm: `ComponentType::FullText => "fulltext.txt"`

- [x] **4.6 Update unit tests in `component.rs`**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/component.rs:170-202`
  - Add test assertion for `determine_component_key` with `ComponentType::FullText`:
    ```rust
    assert_eq!(
        determine_component_key(pmc_id, ComponentType::FullText, prefix),
        "papers/PMC12345/fulltext.txt"
    );
    ```

### Layer 5: Worker Content-Type Mapping (`cortexmap-fetcher`)

- [x] **5.1 Add `ContentType` mapping for `FullText` in the worker**
  - File: `fetcher-be/crates/cortexmap-fetcher/src/worker.rs:100-104`
  - Add match arm: `ComponentType::FullText => ContentType::Text`
  - **Rationale:** `fulltext.txt` is plain text, same MIME type as abstracts.

### Layer 6: Task Enqueue — Register FullText as Default Component (`std-infra`)

- [x] **6.1 Add `ComponentType::FullText` to the enqueue component list**
  - File: `fetcher-be/crates/std-infra/src/task_queue.rs:70-74`
  - Change the `components` vector from:
    ```rust
    let components = vec![
        ComponentType::Summary,
        ComponentType::Abstract,
        ComponentType::Pdf,
    ];
    ```
    to:
    ```rust
    let components = vec![
        ComponentType::Summary,
        ComponentType::Abstract,
        ComponentType::Pdf,
        ComponentType::FullText,
    ];
    ```
  - **Rationale:** This ensures every newly-enqueued task creates a `fulltext` component row in the DB, which the worker will pick up and process.

- [x] **6.2 Add fulltext queries to `get_component_stats()`**
  - File: `fetcher-be/crates/std-infra/src/task_queue.rs:490-546`
  - Add queries for `fulltext_completed` and `fulltext_failed` counts, mirroring the existing summary/abstract/pdf pattern. Filter on `component_type.eq("fulltext")`.
  - Map these to the new `ComponentStats` fields added in step 2.2.

### Layer 7: Proto & Server Updates (monitoring visibility)

- [x] **7.1 Add fulltext fields to `ComponentStatistics` proto message**
  - File: `proto/app/queue.proto:117-125`
  - Add two new fields:
    ```protobuf
    int64 total_fulltext_completed = 8;
    int64 total_fulltext_failed = 9;
    ```
  - **Rationale:** The frontend and monitoring tools use this proto message to display component-level stats. Without these fields, fulltext stats would be invisible.

- [x] **7.2 Populate the new proto fields in the server handler**
  - File: `fetcher-be/crates/cortexmap-be/src/server.rs:296-304`
  - Add the new fields to the `ComponentStatistics` construction:
    ```rust
    total_fulltext_completed: component_stats.fulltext_completed,
    total_fulltext_failed: component_stats.fulltext_failed,
    ```

### Layer 8: Orch Completion Watcher (verification — no code change needed)

- [x] **8.1 Verify `completion_watcher.rs` automatically includes `fulltext.txt`**
  - File: `orch/crates/services/src/completion_watcher.rs:306-312`
  - The existing filter logic is:
    ```rust
    let text_s3_keys: Vec<String> = all_s3_keys
        .iter()
        .filter(|key| {
            let lower_key = key.to_lowercase();
            !lower_key.ends_with(".pdf")
        })
        .cloned()
        .collect();
    ```
  - `fulltext.txt` does NOT end with `.pdf`, so it will pass through this filter automatically. **No code change required.** This is a verification-only step.

## Verification Criteria

- `cargo build` succeeds across all workspace crates after all changes
- `cargo test` passes in `cortexmap-fetcher` (existing tests + new `fulltext` tests + updated `component` tests)
- Database migration applies cleanly: `diesel migration run` succeeds
- Enqueueing a new task creates **4** component rows (summary, abstract, pdf, fulltext) in `fetch_task_components`
- Processing a task with an available full-text PMC article uploads `fulltext.txt` to S3 at `{prefix}/{PMCID}/fulltext.txt`
- The `/api/queue/status` endpoint returns `total_fulltext_completed` and `total_fulltext_failed` in its JSON response
- The orch `completion_watcher` includes `fulltext.txt` S3 keys in the text keys sent to brainatlas `/api/process`
- Articles without a `<body>` section (not all PMC articles have full text) gracefully fail the fulltext component without affecting other components

## Potential Risks and Mitigations

1. **Database CHECK constraint blocks inserts before migration runs**
   Mitigation: The migration (step 1.1) MUST be applied before deploying the code. Document this in deployment order. If using rolling deploys, the migration is backward-compatible (old code never inserts `fulltext`, so the expanded constraint is harmless).

2. **Not all PMC articles have full-text `<body>` sections**
   Mitigation: The `extract_body_from_xml` function returns `FetchError::NotFound` when `<body>` is absent. The existing `handle_component_failure` logic in `worker.rs:230-318` will mark the component as failed after max retries — this is the same behavior as when a PDF is unavailable. The task still completes if other components succeed (fulltext failure is independent).

3. **Full-text content may be very large (some papers are 50+ pages)**
   Mitigation: The text is streamed to S3 via `into_byte_stream()`, though for `FullText(String)` the entire text is first materialized in memory (same as `Abstract`). For extremely large papers, this is still manageable as plain text is compact (~100KB even for long papers). If memory becomes a concern in the future, the fetch could be refactored to stream.

4. **XML body parsing may miss content in non-standard PMC XML structures**
   Mitigation: PMC XML follows the JATS (Journal Article Tag Suite) standard, which consistently uses `<body>` as the container for article content. The tag-stripping approach is intentionally permissive — it extracts all text content even from unknown tags. Unit tests should cover the most common JATS patterns.

5. **Existing tasks in the queue won't have a `fulltext` component row**
   Mitigation: Only newly-enqueued tasks (post-migration) will have the fulltext component. Existing completed tasks are unaffected. If re-processing is desired, tasks would need to be re-enqueued. The `ON CONFLICT DO NOTHING` clause at `task_queue.rs:84-88` means re-enqueueing will not duplicate component rows, but it also means the new `fulltext` row won't be added to existing tasks on re-enqueue (since the task already exists). Consider a one-time data migration script if fulltext is needed for historical tasks.

6. **Proto field number conflicts if other branches add fields concurrently**
   Mitigation: Field numbers 8 and 9 in `ComponentStatistics` are currently unused. Coordinate with other developers to avoid collisions.

## Alternative Approaches

1. **Reuse `fetch_abstract` with a configurable tag name** — Instead of a separate `fetch_fulltext` function, generalize `fetch_abstract` to accept a target tag (`<abstract>` or `<body>`). Trade-off: simpler code but the body extraction needs significantly more tag handling (figures, tables, cross-references) than abstract extraction, making a shared function awkward.

2. **Use a proper XML parser (e.g., `quick-xml` or `roxmltree`) instead of string manipulation** — Would be more robust for parsing JATS XML. Trade-off: adds a new dependency to `cortexmap-fetcher`; the existing codebase uses string-based XML parsing consistently (see `metadata.rs:208-260` and `pdf.rs:88-144`), so this would break the pattern and increase complexity for this change. Could be done as a follow-up refactor.

3. **Store fulltext as Markdown (`.md`) instead of plain text (`.txt`)** — Would preserve section structure better. Trade-off: the brainatlas processing pipeline would need to handle Markdown-formatted content; `.txt` is simpler and aligns with the abstract file format. Could add a separate `fulltext.md` variant later.

4. **Fetch body text via a separate API call instead of reusing EFETCH** — Some PMC articles are available via the PMC OA bulk download. Trade-off: would require a new API integration; EFETCH already returns the body XML in the same response that provides abstracts, so reusing it is zero additional API calls.

## File Change Summary

| File | Change Type | Description |
|---|---|---|
| `fetcher-be/migrations/.../up.sql` | **New** | ALTER CHECK constraint to include 'fulltext' |
| `fetcher-be/migrations/.../down.sql` | **New** | Revert CHECK constraint |
| `fetcher-be/crates/cortexmap-infra/src/infra.rs` | **Modify** | Add `FullText` to `ComponentType` + update `ComponentStats` |
| `fetcher-be/crates/cortexmap-fetcher/src/fetch/fulltext.rs` | **New** | `fetch_fulltext()` + `extract_body_from_xml()` + tests |
| `fetcher-be/crates/cortexmap-fetcher/src/fetch/mod.rs` | **Modify** | Add `pub mod fulltext` |
| `fetcher-be/crates/cortexmap-fetcher/src/fetch/metadata.rs` | **Modify** | Make `EFETCH_URL` and `strip_pmc_prefix` `pub(crate)` |
| `fetcher-be/crates/cortexmap-fetcher/src/component.rs` | **Modify** | Add `FullText` variant to `ComponentResult` + all match arms + tests |
| `fetcher-be/crates/cortexmap-fetcher/src/worker.rs` | **Modify** | Add `ContentType::Text` mapping for `FullText` |
| `fetcher-be/crates/cortexmap-fetcher/src/lib.rs` | **Modify** | Export `fetch_fulltext` |
| `fetcher-be/crates/std-infra/src/task_queue.rs` | **Modify** | Add `FullText` to enqueue + fulltext stats queries |
| `proto/app/queue.proto` | **Modify** | Add fulltext fields to `ComponentStatistics` |
| `fetcher-be/crates/cortexmap-be/src/server.rs` | **Modify** | Populate fulltext stats in status response |
| `orch/crates/services/src/completion_watcher.rs` | **None** | Already handles fulltext.txt (verification only) |

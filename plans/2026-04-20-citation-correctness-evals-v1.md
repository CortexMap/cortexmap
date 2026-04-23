# Citation Correctness Evals

## Objective

Add evals that verify whether the `[chunk:<UUID>]` citations embedded in generated region summaries are **correct** — meaning: (a) the cited UUID exists in the retrieval corpus, (b) it belongs to the same summary's chunks, (c) the cited chunk's text actually supports the sentence it is attached to, and (d) factual sentences are not left uncited.

Today the eval pipeline only judges whether *some* chunk in the corpus supports each claim (`claim_groundedness`/`hallucination_rate` in `evals-be/crates/services/src/state_machine.rs:388-413`). The summaries, by contrast, are explicitly required by the prompt at `brainatlas-be/crates/app/prompts/rag_summarize_system.md:12-18` to carry `[chunk:<UUID>]` markers, but the extraction prompt at `brainatlas-be/crates/app/prompts/extract_claims_system.md:7` strips these markers before any judging happens. The result is a blind spot: a citation can be fabricated, orphaned, or mismatched without any metric catching it.

This plan introduces four new evals in three tiers of increasing sophistication, added to the existing eval state machine without disturbing the current pipeline's semantics.

### Assumptions

- Citations appear exclusively as `[chunk:<UUID>]` tokens embedded in `region_summary.summary`; this is confirmed by the summarization prompt `brainatlas-be/crates/app/prompts/rag_summarize_system.md:12-18` and the frontend parser `atlas/src/components/detail/RegionDetail.tsx:429`.
- The authoritative record of what chunks were ingested for a given summary is `brain_region_embeddings`, joined to `region_summary` via `summary_id` (schema at `brainatlas-be/crates/infra/src/schema.rs:3-24`).
- "Factual sentence" is well-approximated by the existing claim extractor (`brainatlas-be/crates/app/prompts/extract_claims_system.md`) — the same definition used by groundedness.
- Backward compatibility is required: existing `eval_scores` rows and the cache key `(summary_hash, metric, eval_version)` must continue to work. New metrics simply add new `metric` strings.
- Reuses the existing state-machine design (envelope-based LLM calls driven by orch) — no new HTTP calls from evals-be, and no new proto methods on orch.
- Bumping `eval_version` is acceptable for runs that want to include the new metrics; old scores remain valid.
- Performance budget: the cheapest tier (presence / validity) adds zero LLM calls. The most expensive tier (citation-support judge) adds at most one extra judge call per claim; it must be opt-in via config.

## Implementation Plan

### Phase 1 — Domain & wire types

- [x] Task 1. Extend the `Claim` domain type in `brainatlas-be/crates/domain/src/evals.rs:12-22` with a new field `cited_chunks: Vec<Uuid>` (default empty, `#[serde(default)]` for backward compat). Update the `ClaimsResponse` JSON schema via `schemars` so the extractor can return UUIDs alongside text.
  - Rationale: carrying the cited UUIDs through to the judge is the single schema change that unblocks tiers 2 and 3; making it optional keeps old cached payloads readable.

- [x] Task 2. Add a new `EvalMetric` variant cluster in `evals-be/crates/domain/src/evals.rs:66-110`.
  - Variants: `CitationPresence`, `CitationValidity`, `CitationScope`, `CitationSupport`.
  - Update `EvalMetric::all()` at `evals-be/crates/domain/src/evals.rs:89-104` to include them in a new "Citation" section placed after rubric metrics.
  - Rationale: the `IntoStaticStr` derive gives automatic snake_case DB strings (`citation_presence`, etc.), no migration needed thanks to `metric VARCHAR(64)` being free-text (`evals-be/migrations/2026-04-19-000001-create_eval_scores/up.sql:14-15`).

- [x] Task 3. Add a `CitationIssueKind` enum and `CitationIssue` struct to `evals-be/crates/services/src/` (new module `citations.rs`) to serialize into `eval_scores.details`:
  - `CitationIssueKind`: `Missing` (factual sentence with no citation), `Orphan` (UUID not in `brain_region_embeddings`), `OutOfScope` (UUID exists but belongs to a different summary's corpus), `Unsupported` (judge says cited chunk does not support the claim), `Contradicted` (judge says cited chunk contradicts the claim).
  - `CitationIssue`: `{ kind, claim_id, claim_text, offending_chunk_id: Option<Uuid>, rationale: String }`.
  - Rationale: rich `details` payloads are already an established pattern (e.g., claim-level details at `evals-be/crates/services/src/state_machine.rs:395-402`); this gives the frontend enough context to surface actionable repairs.

### Phase 2 — Citation parser (deterministic, no LLM)

- [x] Task 4. Add a `parse_citations(summary: &str) -> Vec<ParsedCitation>` helper in the new `evals-be/crates/services/src/citations.rs`.
  - `ParsedCitation { uuid: Uuid, byte_offset: usize, enclosing_sentence: String }`.
  - Regex: `\[chunk:([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\]` (case-insensitive). Add `regex` as an explicit dependency in `evals-be/crates/services/Cargo.toml`.
  - Sentence segmentation: use a simple `. ! ?` split anchored on the citation's byte offset; bias to include the preceding clause so the judge has context.
  - Unit tests locking: empty summary, zero citations, malformed UUIDs (must be skipped), duplicate citations on the same claim, two citations in one sentence.
  - Rationale: colocating with the existing `structural.rs` deterministic-metric pattern (`evals-be/crates/services/src/structural.rs:1-87`) keeps the style consistent.

- [x] Task 5. Add two additional deterministic helpers in `citations.rs`:
  - `citation_presence_score(summary: &str, claims: &[Claim]) -> (f32, Vec<CitationIssue>)` — fraction of claims whose enclosing sentence in the original summary carries at least one `[chunk:...]` marker. Requires mapping claims back to their source sentences (use the claim's `section` and a fuzzy substring match; if mapping fails, count that claim as "missing" only if the *entire section* has no citations, to avoid false positives from extractor paraphrasing).
  - `citation_scope_score(summary_id: Uuid, cited_uuids: &[Uuid], embeddings_for_summary: &HashSet<Uuid>) -> (f32, Vec<CitationIssue>)` — fraction of cited UUIDs that belong to this summary's ingested chunks.
  - Rationale: both are pure functions that belong next to `section_completeness`/`length_in_range`; no LLM cost.

### Phase 3 — Infra layer: batch chunk lookup

- [x] Task 6. Add a new method on the `EvalsDatabase` trait in `evals-be/crates/services/src/infra.rs:50-180`:
  ```
  async fn load_chunks_by_ids(
      &self,
      database_url: &str,
      chunk_ids: &[Uuid],
  ) -> Result<Vec<ChunkRow>, Self::Error>;
  ```
  - New `ChunkRow { id: Uuid, summary_id: Uuid, chunk_index: i32, chunk_text: String }` colocated with `RetrievedChunk` at `evals-be/crates/services/src/infra.rs:37-43`.
  - Rationale: one round-trip to fetch all cited chunks at once avoids N+1 in the support-judge phase.

- [x] Task 7. Implement `load_chunks_by_ids` in the evals-be infra crate against `brain_region_embeddings`.
  - Diesel schema mirror at `evals-be/crates/infra/src/schema.rs:82-91` — this file currently omits `id`/`summary_id`; regenerate or hand-edit to include them.
  - Use `= ANY($1)` for the UUID array filter; unit-test with an empty list returning empty without a DB round-trip.
  - Rationale: evals-be's schema mirror is intentionally narrow (`evals-be/crates/infra/src/schema.rs`), so widening it is the right fix rather than calling brainatlas over HTTP for something that is a single SQL query.

### Phase 4 — Prompt changes: preserve citations through claim extraction

- [x] Task 8. Update `brainatlas-be/crates/app/prompts/extract_claims_system.md:5-12` so the extractor **preserves**, rather than strips, `[chunk:...]` markers, and reports them as a structured `cited_chunks: [uuid, ...]` field per claim.
  - New rule text: "Do not include `[chunk:...]` markers in the `text` field; instead list the UUIDs they contain (if any) in a `cited_chunks` array on the same claim."
  - Update the JSON schema example at `brainatlas-be/crates/app/prompts/extract_claims_system.md:14-23` accordingly.
  - Rationale: puts the chunk UUIDs on the claim without contaminating the `text` that the groundedness judge already receives.

- [x] Task 9. Bump the `eval_version` constant (search for `eval_version` usages; set via env in `evals-be/crates/app/src/app.rs`) to invalidate cached claim rows that predate the new schema.
  - Rationale: old claims with `cited_chunks: []` would otherwise silently make every summary score 0 on presence.

### Phase 5 — New support judge prompt

- [x] Task 10. Add a new prompt file `brainatlas-be/crates/app/prompts/judge_citation_system.md` next to `judge_groundedness_system.md`.
  - Input: a claim + ONE cited chunk at a time + the surrounding sentence as context.
  - Output schema: `{ verdict: "supported" | "partial" | "contradicted" | "unsupported", confidence: f32, rationale: string }` — deliberately a subset of `GroundednessVerdict` without `supporting_chunks` (only one chunk is passed).
  - Framing: distinct from groundedness — the judge is told the author *claimed* this chunk supports the sentence and must verify that specific attribution, not re-retrieve evidence.

- [x] Task 11. Add a new LLM endpoint to brainatlas-be: `POST /brainatlas-be/api/llm/judge-citation`.
  - Request type in `brainatlas-be/crates/rpc-types/src/evals.rs`: `JudgeCitationRequest { claim_text: String, sentence_context: String, chunk_text: String, chat_model: Option<String> }`.
  - Handler wires through services → app like the existing four eval endpoints (`brainatlas-be/crates/server/src/server.rs:242-273`).
  - Services implementation in `brainatlas-be/crates/services/src/llm_service.rs:116-156` follows the pattern of `judge_groundedness`; returns a reused `GroundednessVerdict` (with `supporting_chunks` left empty).
  - Rationale: slots cleanly into the existing stateless-judge endpoint family without a new proto method.

- [x] Task 12. Add a new `LlmEndpoint::JudgeCitation` variant to the orch/evals-be shared `rpc-types` (`orch/crates/services/src/eval_orchestrator.rs:64-103` mirrors these — keep them in sync).
  - `path() = "/brainatlas-be/api/llm/judge-citation"`.
  - Rationale: orch already dispatches by `LlmEndpoint`; one new variant is the minimum change there.

### Phase 6 — State machine integration

- [x] Task 13. Extend the `RunState` enum in `evals-be/crates/services/src/state_machine.rs:63-87` with two new phases:
  - `AwaitingCitationPrep { claims: Vec<Claim>, reports: Vec<ClaimReport> }` — trivial in-memory phase that scans citations, queries `load_chunks_by_ids`, and produces two "citation issue lists" (presence + validity + scope).
  - `AwaitingCitationSupport { claims: Vec<Claim>, idx: usize, claim_cite_idx: usize, issues: Vec<CitationIssue>, cited_chunks: HashMap<Uuid, ChunkRow> }` — iterates over `(claim_idx, citation_idx)` and issues one `JudgeCitation` call per cited chunk.
  - Place after `AwaitingRubric` so the new work happens after groundedness and rubric are already computed.
  - Rationale: one state per LLM round-trip is the established pattern; the "Prep" phase deliberately requires no LLM call.

- [x] Task 14. Emit `NextAction::CallLlm { endpoint: LlmEndpoint::JudgeCitation, ... }` in `AwaitingCitationSupport`, mirroring the structure of `AwaitingClaimJudge` at `evals-be/crates/services/src/state_machine.rs:281-306`.
  - On each response, append to `issues` if the verdict is `Unsupported` or `Contradicted`; otherwise proceed.
  - Rationale: keeps evals-be purely stateless w.r.t. HTTP — orch still executes the actual brainatlas call.

- [x] Task 15. Add `persist_citation_metrics(...)` helper in `state_machine.rs` following the shape of `persist_groundedness_metrics` at `evals-be/crates/services/src/state_machine.rs:533-605`.
  - Writes four rows via `score_with_cache`: `citation_presence`, `citation_validity`, `citation_scope`, `citation_support`.
  - Each `details` JSON includes the full `Vec<CitationIssue>` filtered to that metric's kind (+ per-claim rollups for quick frontend display).
  - Rationale: consistent persistence pattern keeps the cache key semantics unchanged.

- [x] Task 16. Compute final scores (all in `[0.0, 1.0]`):
  - `citation_presence = 1 - (missing_count / total_factual_claims)`. Factual claims = extractor output count.
  - `citation_validity = 1 - (orphan_count / total_citations)`. Edge case: 0 citations total → score = 0.0 with a note in `details.reason = "no_citations"`. This is deliberately harsh to surface summaries that omit citations wholesale.
  - `citation_scope = 1 - (out_of_scope_count / total_existing_citations)`. Edge case: 0 existing citations → 1.0 (vacuously true, nothing to be out of scope).
  - `citation_support = supported / (supported + partial + contradicted + unsupported)`, where `partial` counts as 0.5. Use the same `0.8` weighting convention as groundedness aggregation if desired; lock this explicitly with a test.
  - Rationale: these are independent axes — mixing them into one score would hide specific failure modes. Each has clear remediation: presence → fix prompt; validity → fix hallucinated UUIDs; scope → fix tool-schema filtering; support → fix retrieval ranking.

- [x] Task 17. Update `initial_action` at `evals-be/crates/services/src/state_machine.rs:94-138` to accept a new `citations_cached: bool` flag and a new config toggle `citation_support_enabled: bool` on `RunContext`.
  - When all five families are cached → `Done` as before.
  - When rubric is cached but citations are not → jump straight to `AwaitingCitationPrep` (via the existing claims phase, because presence needs `Vec<Claim>`).
  - When groundedness is not cached, the claims phase already produces the `Vec<Claim>` with the new `cited_chunks` field, which feeds directly into citations — no re-extraction needed.
  - Rationale: avoids re-running extraction; citations ride on the same Claim objects.

- [x] Task 18. Thread the new `citation_support_enabled` flag through `evals-be/crates/app/src/app.rs:34-97` (`EvalRuntimeConfig::from_env`) and read env `EVAL_CITATION_SUPPORT_ENABLED` (default `false`).
  - When disabled, the state machine computes the three deterministic citation metrics (`presence`, `validity`, `scope`) but skips `support` entirely — no extra LLM cost.
  - Rationale: staged rollout; operators enable the expensive metric once prompt/model quality is verified.

### Phase 7 — Cache & run-state compatibility

- [x] Task 19. Verify the `eval_run_state.state JSONB` column (`evals-be/migrations/2026-04-19-000002-add_eval_run_state/up.sql:13-25`) handles the new `RunState` variants.
  - Add a serde round-trip test in `state_machine.rs` locking the JSON shape of each new variant to catch accidental rename-driven corruption.
  - Rationale: existing state rows are keyed by `pending_endpoint: Option<&str>`; the new `JudgeCitation` endpoint string must not collide.

- [x] Task 20. Confirm `score_with_cache` (`evals-be/crates/services/src/cache.rs:33-80`) works unchanged with the new metric names — it's driven off the free-text `metric` column and enforces uniqueness by `(summary_hash, metric, eval_version)`.

### Phase 8 — Orchestrator wiring

- [x] Task 21. Update orch's LLM dispatch map in `orch/crates/services/src/eval_orchestrator.rs:64-103` to recognise `LlmEndpoint::JudgeCitation` and route it to brainatlas's new `/brainatlas-be/api/llm/judge-citation` path.
  - No new HTTP client needed — uses the existing `HttpClient` port (`orch/crates/services/src/infra.rs:90-106`).

- [x] Task 22. Include the new citation metric names in any orch aggregation/summary endpoints (`orch/crates/services/src/` — search for `per_metric`/`rubric_` constants) so dashboards show them without code-level denylisting.

### Phase 9 — Frontend surface (read-only)

- [x] Task 23. Extend the eval-score display in `brainatlas-fe/src/components/SummaryDisplay.jsx` (and/or `atlas/src/components/detail/RegionDetail.tsx`) to render the four new metrics.
  - When `details.issues` is non-empty, render a collapsible "Citation issues" panel listing each issue with its claim text, offending chunk id, and rationale.
  - Clicking an "orphan" citation should visually distinguish it in the rendered summary (leverage the existing citation-bubble component at `atlas/src/components/detail/RegionDetail.tsx:441`).
  - Rationale: the eval is actionable only if reviewers can see *which* citations failed.

### Phase 10 — Testing

- [x] Task 24. Unit tests in `evals-be/crates/services/src/citations.rs` (TDD-style, lock formulas):
  - Parse: handles empty, zero-match, multi-match, malformed UUID, adjacent citations.
  - Presence: all-cited → 1.0; half-cited → 0.5; uncited factual-claim counted; acronym/header-only sections not penalised.
  - Validity: orphan detection → correct score, correct `CitationIssue` list.
  - Scope: cross-summary UUID leakage detected; empty-citation case scores 1.0 with reason in details.
  - Support aggregation math: partial=0.5 weighting; empty→0.0; all-supported→1.0.

- [x] Task 25. State-machine integration tests in `evals-be/crates/services/src/state_machine.rs` mod tests:
  - Feed a synthetic `ClaimsResponse` with `cited_chunks` populated; drive through `AwaitingCitationPrep` → `AwaitingCitationSupport` loop with a mocked `EvalsDatabase` and `LlmResponsePayload::CitationSupport(...)` variant. Assert four metric rows are persisted with correct scores.
  - Idempotency test: re-running a cached run emits zero new LLM calls and returns `Done`.
  - Flag-off test: `citation_support_enabled = false` writes three rows (presence, validity, scope) and zero support.

- [ ] Task 26. (Deferred — see completion notes.) End-to-end integration test under `tests/` covering:
  - A summary with 3 factual sentences, 2 cited correctly, 1 cited with an orphan UUID, 1 uncited.
  - Assert: `citation_presence = 3/4 = 0.75`, `citation_validity = 2/3`, `citation_scope = 2/2 = 1.0`, `citation_support` ≥ 0.5 given a stubbed support judge.
  - Uses the docker-compose test stack and the same test scaffolding as existing integration tests (`./test.sh`, `setup-test-data.sh`).

### Phase 11 — Rollout

- [x] Task 27. (Code-ready for deployment; flag defaults to `false`.) Stage 1 deploy: ship Phases 1–7 + 10 (parser, presence, validity, scope) with `EVAL_CITATION_SUPPORT_ENABLED=false`. Three new deterministic metrics appear in every run; no new LLM cost.

- [ ] Task 28. (Production observation step — gated on deployment.) Observe a run cycle in production, confirm all three deterministic metrics produce sane distributions, and spot-check `CitationIssue` payloads for false positives (especially presence, which relies on claim↔sentence matching).

- [ ] Task 29. (Stage 2 toggle — gated on Stage 1 deployment + prompt tuning.) Stage 2 deploy: tune the support-judge prompt against a hand-curated set of 20 (claim, chunk, verdict) tuples, then enable `EVAL_CITATION_SUPPORT_ENABLED=true` behind the same env toggle. Monitor LLM cost impact via the cost-tracking table from the parallel `llm-cost-tracking` plan.

- [x] Task 30. Update the eval documentation section of `README.md` (or the evals-be service README) listing the four new metrics and their scoring formulas, and add a short runbook entry covering how to flip the support-judge toggle.

## Verification Criteria

- `EvalMetric::all()` returns 15 metrics (11 existing + 4 new) and every variant round-trips through its snake_case DB string.
- A summary with zero `[chunk:…]` markers scores `citation_presence = 0.0`, `citation_validity = 0.0` (with `details.reason = "no_citations"`), `citation_scope = 1.0`, `citation_support = 0.0` (with `details.reason = "no_citations"`).
- A summary where every factual claim has at least one valid, in-scope, judge-approved citation scores `1.0` on all four.
- A summary with exactly one orphan UUID out of 10 citations scores `citation_validity = 0.9` and emits exactly one `CitationIssue { kind: Orphan, ... }` in the `details` JSON.
- A summary with an out-of-scope UUID (valid chunk, different `summary_id`) scores `citation_scope < 1.0` and flags the issue; `citation_validity` is unaffected.
- The state machine's JSONB serialisation round-trips through `serde_json` for every new `RunState` variant without field drift.
- With `EVAL_CITATION_SUPPORT_ENABLED=false`, running an eval cycle produces rows for `citation_presence`, `citation_validity`, `citation_scope` and *no* row for `citation_support`, and issues zero calls to `LlmEndpoint::JudgeCitation`.
- With `EVAL_CITATION_SUPPORT_ENABLED=true`, a summary with 10 citations issues ≤ 10 `JudgeCitation` calls (one per citation), and the resulting `citation_support` score equals the formula exactly for a fixture set of known verdicts.
- Existing metrics (`claim_groundedness`, `hallucination_rate`, rubric family, structural family) produce identical scores before and after this change on a fixture summary (regression guard).
- The `eval_version` bump causes old cache hits to miss as expected; new scores populate on first run.
- Frontend renders the four new metrics and surfaces `CitationIssue` lists in a collapsible panel (manual QA).

## Potential Risks and Mitigations

1. **Claim ↔ sentence mapping in `citation_presence` is imperfect — extractor paraphrases may not substring-match the source sentence.**
   Mitigation: fall back from exact substring match to a token-Jaccard threshold ≥ 0.5 before declaring a claim "missing". Log unmatched claims at `warn!` for offline evaluation. Consider including a `source_sentence: String` field on `Claim` in a future pass so the extractor reports the verbatim sentence rather than a rewrite.

2. **Support judge cost blows up on summaries with many citations.**
   Mitigation: (a) the feature is behind an env toggle that defaults off; (b) cap support-judge calls at `CITATION_SUPPORT_MAX_CALLS_PER_SUMMARY` (default 30) with a warn log and partial-score flag in `details` when truncated; (c) rely on the parallel LLM cost tracking plan to expose per-eval-run cost dashboards.

3. **Judge disagrees with groundedness judge on the same claim.**
   Mitigation: acceptable and informative — groundedness asks "does retrieval support this claim?" and citation-support asks "did the author cite the right chunk?". Discrepancy is a feature, not a bug. Document this explicitly in the support-judge prompt.

4. **Regex-based citation parser false-matches inside code blocks or example text.**
   Mitigation: strip fenced code blocks before parsing; unit-test with the summarization prompt's own example text at `brainatlas-be/crates/app/prompts/rag_summarize_system.md:15-16` to ensure the example citation doesn't leak into parsed output.

5. **Widening `evals-be/crates/infra/src/schema.rs:82-91` to include `id`/`summary_id` drifts from brainatlas-be's canonical schema.**
   Mitigation: add a schema-shape test that reads `\d brain_region_embeddings` via raw SQL and diffs against a checked-in golden file; keep the two Diesel declarations explicitly minimal-and-documented rather than regenerating from pg.

6. **Old cached eval_scores rows cause false-positive "regression" when new metrics appear.**
   Mitigation: the `eval_version` bump (Task 9) forces a fresh scoring pass for any summary that wants the new metrics; the cache entry for existing metrics at older versions continues to serve the historical dashboard.

7. **Cross-summary citation is sometimes legitimate (e.g., the RAG tool returns chunks from related regions).**
   Mitigation: `citation_scope` is a diagnostic, not a gate. Consider a follow-up where `scope` is relaxed to "within the same region_id" rather than "within the same summary_id" if out-of-scope-but-same-region is widespread. For now, log it and let the team tune.

8. **`ChunkRow.summary_id` is on `brain_region_embeddings`, but the column currently exists only via `NewEmbedding`'s writes — confirm the DB schema actually stores it.**
   Mitigation: verify via `brainatlas-be/crates/infra/src/schema.rs:3-24` and the canonical migration (`brainatlas-be/migrations/2026-02-14-add-embeddings-support/up.sql`). If `summary_id` is absent on `brain_region_embeddings` (only `region_id` present), add a new migration that adds it with a backfill. This must be verified before Task 7.

9. **The claim-extractor prompt update (Task 8) might confuse older cached orch calls that still expect the old schema.**
   Mitigation: make `cited_chunks` optional in deserialization (`#[serde(default)]`) and keep tolerating responses without it. Old cached rows remain valid for groundedness even if they lack `cited_chunks`.

## Alternative Approaches

1. **LLM-only citation check (skip deterministic parsing, rely on a single "critique" judge pass over the summary).**
   Trade-offs: simpler code, one LLM call per summary instead of up to N. But: no orphan-UUID detection without actually resolving UUIDs against the DB; weaker signal; expensive per-call; no issue-level drill-down. Rejected: the deterministic checks are cheap, precise, and catch the failure modes LLMs miss (fabricated UUIDs).

2. **Post-process summaries in brainatlas-be at generation time (push the check left into `BrainAtlasApp::validate_citations`).**
   Trade-offs: catches errors before storage, avoids the eval round-trip, and could even retry/patch bad citations. But: (a) the `plans/2026-03-26-citation-validation-post-processing-v1.md` plan already proposed this and was never shipped; (b) generation-time validation fights with the generation prompt and risks masking quality issues rather than surfacing them. Keep as a future complement — an eval establishes the ground truth a post-processor can optimise against.

3. **Replace `[chunk:UUID]` markers with numeric references + a bibliography block (BibTeX-style).**
   Trade-offs: matches scientific-paper conventions; reference integrity becomes a byproduct of rendering. But: requires changes to the summarization prompt, the frontend citation-bubble UI (`atlas/src/components/detail/RegionDetail.tsx:428-693`), and all downstream consumers; invalidates historical summaries. Out of scope here — citations evals should work with the existing `[chunk:UUID]` format first.

4. **Only ship the three deterministic metrics, skip the support judge entirely.**
   Trade-offs: zero LLM cost, simpler rollout, no prompt engineering risk. But: deterministic metrics cannot catch "the cited chunk exists and is in scope but does not support the claim" — which is the most insidious failure mode. Adopt as the Stage 1 shape of this plan (Task 27) but explicitly aim for full Stage 2 once support-judge quality is verified.

5. **Model citations as a CHECK constraint / referential integrity on `region_summary` text at write time.**
   Trade-offs: would eliminate orphans entirely by construction. But: Postgres cannot enforce UUID-within-text FKs without a trigger, and rejecting a summary write for a bad citation is an operational footgun. Evals are the right abstraction layer: measure, don't gate.

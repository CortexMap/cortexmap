# Auto-Queue Latest Eval for Newly Generated Summaries

## Objective

When orch successfully triggers summary generation, it should automatically enqueue an eval run for the returned summary, while keeping orch as the component that owns cross-service orchestration.

The smallest safe v1 is:

- expose the generated `summary_id` explicitly from brainatlas-be’s `/api/process` and `/api/process-no-papers` responses,
- update orch’s response mirrors to consume that field,
- have orch call the existing eval queue endpoint immediately after a successful summary-generation flow,
- keep eval queueing non-blocking with respect to summary creation so eval outages do not retroactively mark a successfully created summary as failed.

## Current-State Summary

### Project Structure Summary

- `brainatlas-be` already knows the generated summary UUID internally. Both app-layer entry points return `Result<Uuid, AppError<_>>`, so the summary identity already exists before the HTTP response is built. Source: `brainatlas-be/crates/app/src/app.rs:97-108`, `brainatlas-be/crates/app/src/app.rs:307-313`. Implication: the missing data is a wire-contract problem, not a core generation-logic problem.
- The public proto/HTTP response for both processing paths exposes only `region_id` and a human-readable `detail` string. Source: `proto/llm/brain.proto:120-147`. Implication: orch cannot safely recover the new summary UUID from typed response data today.
- The brainatlas API layer currently converts the app-layer `summary_id` into a formatted string inside `detail` and drops the structured UUID. Source: `brainatlas-be/crates/api/src/brainatlas_api.rs:73-103`, `brainatlas-be/crates/api/src/brainatlas_api.rs:118-130`. Implication: parsing `detail` would be brittle and should not be the implementation strategy.
- Orch triggers paper-backed summary generation in the completion watcher via `/brainatlas-be/api/process`, and it currently treats success as an opaque `detail` string. Source: `orch/crates/services/src/completion_watcher.rs:547-645`. Implication: this is the primary insertion point for automatic eval queueing of standard batches.
- Orch triggers knowledge-only summary generation in the pipeline runner via `/brainatlas-be/api/process-no-papers`, and it currently discards the response body beyond success/failure. Source: `orch/crates/services/src/pipeline_runner.rs:34-82`, `orch/crates/services/src/pipeline_runner.rs:578-627`. Implication: the no-papers fallback needs the same summary-id exposure and queueing hook.
- Evals are already queue-driven. `POST /evals-be/api/evals/batch` accepts one or more `summary_ids`, optionally an `eval_version`, and upserts queued rows idempotently. Source: `evals-be/crates/rpc-types/src/lib.rs:178-206`, `evals-be/crates/app/src/app.rs:547-598`, `evals-be/crates/api/src/router.rs:40-49`, `evals-be/crates/api/src/router.rs:146-156`. Implication: no new evals-be queueing endpoint is required for v1.
- Orch’s eval orchestrator drains `/evals-be/api/evals/unscored` for the version returned by its own `eval_version()` helper, sourced from orch config. Source: `orch/crates/services/src/eval_orchestrator.rs:214-218`, `orch/crates/services/src/eval_orchestrator.rs:271-286`, `orch/crates/domain/src/lib.rs:62-71`. Implication: queue-time version selection must stay aligned with what orch later polls.
- The broader eval architecture already states that eval failures must not block the main summary pipeline. Source: `plans/2026-04-19-evals-architecture-v1.md:21-25`. Implication: eval enqueue failure should be retried and surfaced, but should not roll back a summary that was already created successfully.

### Relevant File Examination

- `proto/llm/brain.proto:120-147` — authoritative response shape for both processing RPC/HTTP routes.
- `brainatlas-be/crates/api/src/api.rs:30-53` — trait contract showing both process methods return `ProcessRegionResponse` at the API layer.
- `brainatlas-be/crates/api/src/brainatlas_api.rs:58-130` — exact location where `summary_id` is available but not serialized structurally.
- `brainatlas-be/crates/server/src/server.rs:185-237` — handlers simply return `Json(resp)`, so an additive response field should flow through without handler-specific logic.
- `brainatlas-be/crates/rpc-types/src/lib.rs:10-18` — generated protobuf types are re-exported from one place; response-shape changes propagate from the proto.
- `orch/crates/services/src/types.rs:46-78` — orch’s local JSON mirror for `ProcessRegionResponse` currently lacks `summary_id`.
- `orch/crates/services/src/completion_watcher.rs:426-645` — standard summary path and current success boundary.
- `orch/crates/services/src/pipeline_runner.rs:34-82`, `orch/crates/services/src/pipeline_runner.rs:563-627` — knowledge-only summary path and current success boundary.
- `orch/crates/services/src/eval_orchestrator.rs:187-218` — existing helpers for resolving evals base URL and orch’s active eval version.
- `evals-be/crates/rpc-types/src/lib.rs:187-206` — queue request/response contract.
- `evals-be/crates/api/src/api.rs:35-40` and `evals-be/crates/app/src/app.rs:561-598` — existing queue API and semantics.

### Prioritized Challenges and Risks

1. **Structured summary identity is missing on the wire.**
   This is the blocker that prevents a safe orch-side enqueue call. It ranks first because no downstream design works cleanly until `summary_id` is exposed. Sources: `proto/llm/brain.proto:120-147`, `orch/crates/services/src/types.rs:74-78`.
2. **Queue-time eval version can drift from drain-time eval version.**
   Evals-be defaults `batch_eval` to its own configured version, while orch drains unscored rows using orch config. If those ever diverge, queued summaries may appear “missing” to the poller. This ranks second because it can produce a silent operational failure. Sources: `evals-be/crates/app/src/app.rs:575-597`, `orch/crates/services/src/eval_orchestrator.rs:214-218`, `orch/crates/services/src/eval_orchestrator.rs:277-286`.
3. **Summary generation can reuse an existing summary via content-hash dedupe.**
   Both brainatlas generation paths may return an existing `summary_id` instead of creating a new row. This ranks third because the v1 design can tolerate it, but it affects the exact interpretation of “newly created summary.” Sources: `brainatlas-be/crates/app/src/app.rs:182-198`, `brainatlas-be/crates/app/src/app.rs:323-343`.
4. **Eval enqueue failure must not poison the main pipeline.**
   A queue POST can fail after summary creation has already succeeded. This ranks fourth because it is operationally important, but the mitigation is straightforward: retry, log, and do not roll back summary completion. Sources: `orch/crates/services/src/completion_watcher.rs:617-645`, `plans/2026-04-19-evals-architecture-v1.md:21-25`.

## Recommended Implementation Approach

### Recommendation

Recommend an **additive wire-contract change plus orch-side queueing**:

1. Add `summary_id` to `ProcessRegionResponse` and `ProcessNoPapersResponse` in `proto/llm/brain.proto:120-147`.
2. Populate that field in `brainatlas-be/crates/api/src/brainatlas_api.rs:58-130`; no change is required to the underlying app logic because `brainatlas-be/crates/app/src/app.rs:97-108` and `brainatlas-be/crates/app/src/app.rs:307-313` already return the UUID.
3. Update orch’s response mirrors in `orch/crates/services/src/types.rs:74-78` so the completion watcher and pipeline runner receive the UUID structurally.
4. After each successful summary-generation path completes, have orch call the existing eval queue endpoint `POST /evals-be/api/evals/batch` defined in `evals-be/crates/rpc-types/src/lib.rs:187-206`.
5. Pass an **explicit eval version resolved using the same orch helper that the poller uses**, rather than relying on evals-be’s omitted-version default. This keeps queue-time and drain-time semantics aligned with `orch/crates/services/src/eval_orchestrator.rs:214-218` and `orch/crates/services/src/eval_orchestrator.rs:271-286`.
6. Treat eval enqueue as **best-effort with retries and warnings**, not as a reason to mark summary generation failed, consistent with `plans/2026-04-19-evals-architecture-v1.md:21-25`.

### Why this is preferable

- It is the **smallest viable change** that keeps orch as the orchestrator.
- It avoids brittle parsing of `detail` strings built in `brainatlas-be/crates/api/src/brainatlas_api.rs:89-103`, `brainatlas-be/crates/api/src/brainatlas_api.rs:124-130`.
- It reuses the existing, idempotent batch queue contract in `evals-be/crates/app/src/app.rs:561-598`.
- It avoids pushing new cross-service responsibilities into brainatlas-be.
- It keeps v1 focused on auto-enqueueing, without reopening unrelated prompt or scoring work.

## File-by-File Change List

### Files that should change

- `proto/llm/brain.proto:120-147`
  - Add `summary_id` to both `ProcessRegionResponse` and `ProcessNoPapersResponse`.
  - Keep the change additive so existing callers that ignore unknown fields remain unaffected.

- `brainatlas-be/crates/api/src/brainatlas_api.rs:58-130`
  - Populate the new response field from the app-layer `summary_id` already returned by `process_region` and `process_region_no_papers`.
  - Leave the human-readable `detail` intact for backward compatibility.

- `brainatlas-be/crates/rpc-types/src/lib.rs:10-18`
  - Regenerate/re-export the updated protobuf-generated response type so downstream crates see the new field.

- `orch/crates/services/src/types.rs:74-78`
  - Extend orch’s `ProcessRegionResponse` mirror to include `summary_id`.
  - Add a local mirror for the eval queue request/response if needed, rather than introducing a larger cross-workspace dependency.

- `orch/crates/services/src/eval_orchestrator.rs:187-218`
  - Extract or expose a small internal helper for resolving evals base URL and the effective orch eval version.
  - Optionally define a reusable `queue_summary_for_eval(summary_id, eval_version)` helper here so both generation paths call the same code.

- `orch/crates/services/src/completion_watcher.rs:617-645`
  - After a successful `/api/process` call and after the batch is marked complete, queue the returned `summary_id` for eval.
  - Retry the queue POST, log failures with `batch_id`, `region_id`, and `summary_id`, but do not downgrade the batch to failed if queueing still fails.

- `orch/crates/services/src/pipeline_runner.rs:34-82`, `orch/crates/services/src/pipeline_runner.rs:578-627`
  - Capture the knowledge-only `summary_id` from `/api/process-no-papers` and queue it through the same helper after the batch is completed successfully.

- `brainatlas-be/crates/server/tests/handler_test.rs`
  - Add handler-level assertions that `/brainatlas-be/api/process` and `/brainatlas-be/api/process-no-papers` responses include the new `summary_id` field.

- `orch/crates/services/src/completion_watcher.rs:676-900`
  - Extend the fake HTTP/mutation recorder so unit tests can assert that a successful process call is followed by a queue POST to evals-be.

- `orch/crates/services/src/pipeline_runner.rs:912-1796`
  - Add/extend tests around the knowledge-only path to verify the eval queue POST occurs when the summary-generation POST succeeds.

### Files that likely do **not** need code changes for v1

- `brainatlas-be/crates/app/src/app.rs:97-108`, `brainatlas-be/crates/app/src/app.rs:307-313`
  - No semantic change required; these methods already return `Uuid`.

- `brainatlas-be/crates/server/src/server.rs:185-237`
  - Handler logic should not need modification beyond recompilation, because the handlers already serialize the whole response object.

- `evals-be/crates/rpc-types/src/lib.rs:187-206`
  - The existing `BatchEvalRequest` and `BatchEvalResponse` are sufficient.

- `evals-be/crates/api/src/api.rs:35-40`, `evals-be/crates/api/src/router.rs:40-49`, `evals-be/crates/app/src/app.rs:561-598`
  - No new endpoint or app behavior is required if orch reuses `POST /evals-be/api/evals/batch` as-is.

- `orch/crates/app/src/services.rs:255-287`, `orch/crates/services/src/services.rs:433-469`
  - No public trait changes are needed if auto-queueing remains an internal detail of the concrete service implementations.

## Testing Strategy

- **Brainatlas-be handler coverage**
  - Extend `brainatlas-be/crates/server/tests/handler_test.rs` so the process endpoints assert a structured `summary_id` is serialized, not just a `detail` string. This validates the new wire contract at the actual HTTP boundary used by orch.

- **Completion watcher unit coverage**
  - Extend the existing fake infra in `orch/crates/services/src/completion_watcher.rs:676-900` to record outbound POST URLs/bodies.
  - Add a happy-path test proving: `/brainatlas-be/api/process` succeeds, `complete_batch` is called, and then `/evals-be/api/evals/batch` is posted with the returned `summary_id`.
  - Add a degraded-path test proving queue failure is logged/reported but does not convert the already successful summary creation into a failed batch.

- **Knowledge-only pipeline unit coverage**
  - Add a test in `orch/crates/services/src/pipeline_runner.rs:912-1796` proving that the zero-paper fallback path posts `/brainatlas-be/api/process-no-papers`, completes the batch, and then posts `/evals-be/api/evals/batch` with the returned `summary_id`.

- **Version-alignment coverage**
  - Add a unit test around the queue helper to verify the request carries the same eval version that orch’s poller would later drain, based on `orch/crates/services/src/eval_orchestrator.rs:214-218`.

- **Optional integration smoke test**
  - If an integration test is desired later, exercise one end-to-end summary generation cycle and assert that the resulting summary id appears in queued eval runs. This is optional for v1 if the unit/handler coverage above is strong.

## Suggested Execution Order

1. Update the proto response contract in `proto/llm/brain.proto:120-147`.
2. Populate the new field in `brainatlas-be/crates/api/src/brainatlas_api.rs:58-130` and refresh generated rpc types.
3. Update orch’s response mirror in `orch/crates/services/src/types.rs:74-78`.
4. Add the shared orch helper for eval base URL + eval version resolution, reusing the logic in `orch/crates/services/src/eval_orchestrator.rs:187-218`.
5. Hook the helper into the standard paper-backed path in `orch/crates/services/src/completion_watcher.rs:617-645`.
6. Hook the same helper into the knowledge-only path in `orch/crates/services/src/pipeline_runner.rs:34-82` and `orch/crates/services/src/pipeline_runner.rs:578-627`.
7. Add/extend tests in brainatlas-be and orch.
8. Run focused crate tests for `brainatlas-be`, `orch` services, and any affected handler suites.

## Clarity Assessment and Assumptions

- Assumption: v1 should keep orch, not brainatlas-be, responsible for triggering eval queueing, matching the service-boundary intent in `plans/2026-04-19-evals-architecture-v1.md:21-23`.
- Assumption: passing the explicitly resolved orch eval version is safer than omitting the field and relying on evals-be defaults, because orch’s poller already uses orch config (`orch/crates/services/src/eval_orchestrator.rs:214-218`).
- Assumption: if brainatlas-be returns an existing deduplicated summary instead of creating a new row, re-queueing that summary is acceptable in v1 because `batch_eval` is idempotent at `(summary_id, eval_version)` and refreshes the row back to `queued`. Sources: `brainatlas-be/crates/app/src/app.rs:182-198`, `brainatlas-be/crates/app/src/app.rs:323-343`, `evals-be/crates/app/src/app.rs:556-598`.
- Assumption: persisting `summary_id` onto the batch row is out of scope for this feature; the request only needs automatic eval queueing.

## Implementation Plan

- [ ] Task 1 (Status: Not Started). Add `summary_id` to `ProcessRegionResponse` and `ProcessNoPapersResponse` in `proto/llm/brain.proto:120-147`, keeping the change additive. Rationale: orch needs a structured identifier rather than parsing `detail`.
- [ ] Task 2 (Status: Not Started). Populate the new `summary_id` field in `brainatlas-be/crates/api/src/brainatlas_api.rs:58-130` and regenerate the exported rpc types referenced by `brainatlas-be/crates/rpc-types/src/lib.rs:10-18`. Rationale: the app layer already returns the UUID, so the API layer should stop discarding it.
- [ ] Task 3 (Status: Not Started). Extend orch’s response mirror in `orch/crates/services/src/types.rs:74-78` and add a small local eval-batch request mirror if needed. Rationale: orch needs typed access to both the returned `summary_id` and the queue request contract.
- [ ] Task 4 (Status: Not Started). Extract a small internal helper in orch that resolves `evals_base_url` and the effective eval version using the same logic already present in `orch/crates/services/src/eval_orchestrator.rs:187-218`, then POSTs `/evals-be/api/evals/batch`. Rationale: both generation paths should reuse one queueing implementation, and version resolution must match the poller.
- [ ] Task 5 (Status: Not Started). Call the queue helper from the standard batch-completion path in `orch/crates/services/src/completion_watcher.rs:617-645`, after the summary-generation call succeeds and after the batch is marked complete. Rationale: this preserves the existing batch-success boundary while adding the eval side effect at the safest point.
- [ ] Task 6 (Status: Not Started). Call the same queue helper from the knowledge-only fallback path in `orch/crates/services/src/pipeline_runner.rs:34-82` and `orch/crates/services/src/pipeline_runner.rs:578-627`, again only after successful summary creation and batch completion. Rationale: the feature requirement applies to both orch-triggered generation paths.
- [ ] Task 7 (Status: Not Started). Add bounded retries and warning/error logging around the eval queue POST, but do not convert a successfully created summary into a failed generation result if queueing still fails. Rationale: this aligns with the eval architecture’s requirement that eval failures must not block the main summary pipeline (`plans/2026-04-19-evals-architecture-v1.md:21-25`).
- [ ] Task 8 (Status: Not Started). Add handler-level tests in `brainatlas-be/crates/server/tests/handler_test.rs` that assert `summary_id` is present on both process responses. Rationale: the wire-contract change should be locked down where orch actually consumes it.
- [ ] Task 9 (Status: Not Started). Extend `orch/crates/services/src/completion_watcher.rs:676-900` tests to verify that successful paper-backed generation produces an eval queue POST with the returned `summary_id` and aligned `eval_version`. Rationale: this is the primary production path.
- [ ] Task 10 (Status: Not Started). Extend `orch/crates/services/src/pipeline_runner.rs:912-1796` tests to verify that successful knowledge-only generation also produces an eval queue POST with the returned `summary_id`. Rationale: zero-paper regions must not be excluded from automatic eval queueing.

## Verification Criteria

- [ ] A successful `POST /brainatlas-be/api/process` response now includes a structured `summary_id` in addition to `region_id` and `detail`, as defined by `proto/llm/brain.proto:120-124` after the change.
- [ ] A successful `POST /brainatlas-be/api/process-no-papers` response now includes a structured `summary_id` in addition to `region_id` and `detail`, as defined by `proto/llm/brain.proto:143-147` after the change.
- [ ] The standard orch summary path in `orch/crates/services/src/completion_watcher.rs:617-645` issues exactly one eval queue request per successful summary-generation response.
- [ ] The knowledge-only orch summary path in `orch/crates/services/src/pipeline_runner.rs:578-627` issues exactly one eval queue request per successful summary-generation response.
- [ ] The eval queue request uses the same effective eval version that orch’s poller later drains, based on `orch/crates/services/src/eval_orchestrator.rs:214-218` and `orch/crates/services/src/eval_orchestrator.rs:271-286`.
- [ ] If the eval queue POST fails after retries, the summary-generation path still reports success for the already-created summary and emits enough structured logging to identify the missed enqueue.

## Potential Risks and Mitigations

1. **Queue-time and drain-time eval versions diverge across services.**
   Mitigation: do not rely on the omitted-version default in `evals-be/crates/app/src/app.rs:575-597`; instead resolve the version in orch using the same helper the poller uses (`orch/crates/services/src/eval_orchestrator.rs:214-218`) and pass it explicitly.

2. **Content-hash dedupe can return an existing summary instead of creating a new one.**
   Mitigation: accept the idempotent requeue in v1 because `batch_eval` refreshes `(summary_id, eval_version)` back to `queued` rather than creating duplicate workers (`evals-be/crates/rpc-types/src/lib.rs:182-185`, `evals-be/crates/app/src/app.rs:556-598`). If stricter “newly created only” semantics become necessary, add a follow-up `created_new_summary` flag to the response.

3. **Eval queueing fails after summary creation has already succeeded.**
   Mitigation: retry with backoff, log the failure with `summary_id`, `batch_id`, and `region_id`, and keep the generation batch complete rather than failed, consistent with `plans/2026-04-19-evals-architecture-v1.md:21-25`.

4. **Proto response changes affect external callers.**
   Mitigation: keep the response change additive so legacy clients that ignore unknown fields continue to work.

5. **Implementation drift between the paper-backed path and the knowledge-only path.**
   Mitigation: centralize queueing into one internal orch helper rather than duplicating URL/version logic in two places.

## Alternative Approaches

1. **Recommended: additive `summary_id` wire change + orch-side queue call using the existing batch endpoint.**
   Trade-off: requires a small proto/API ripple, but preserves clean ownership boundaries and uses an existing idempotent eval queue.

2. **Smaller but brittle: parse `summary_id` out of the `detail` string currently built in `brainatlas-be/crates/api/src/brainatlas_api.rs:89-103`, `brainatlas-be/crates/api/src/brainatlas_api.rs:124-130`.**
   Trade-off: avoids proto changes, but couples orch to human-readable message text and will break on wording changes.

3. **Have brainatlas-be queue evals directly after summary generation.**
   Trade-off: superficially fewer orch changes, but it violates the desired service boundary by pushing orchestration responsibility into brainatlas-be and couples summary generation to eval infrastructure.

4. **Rely on evals-be’s omitted-version default for `batch_eval`.**
   Trade-off: slightly less orch code, but it risks queueing rows under a version different from the one orch’s poller asks `/unscored` for, based on `orch/crates/services/src/eval_orchestrator.rs:277-286`.

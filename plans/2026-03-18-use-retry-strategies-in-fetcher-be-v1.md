# Use Retry Strategies in fetcher-be

## Objective

Wire up the existing `BackoffStrategy` enum and `backon` crate dependency so that retry behavior is actually applied at two levels:

1. **Request-level retries** -- individual HTTP and S3 calls use `backon` to retry transient failures before consuming a task-level attempt.
2. **Task-level backoff** -- when a component fails and the task is released back to the queue for re-processing, the worker sleeps according to the configured `BackoffStrategy` (constant, linear, exponential, fibonacci) rather than always using the fixed `timeout_secs`.

Additionally, wire up the unused `RetryConfig::get_component_max_retries()` so per-component retry limits are respected.

---

## Current State Analysis

### What exists but is unused:

| Asset | Location | Status |
|-------|----------|--------|
| `BackoffStrategy` enum (4 variants) | `cortexmap-core/.../fetcher.rs:44-72` | Defined, parsed from CLI, logged, **never computed** |
| `backon = "1.3.0"` dependency | `cortexmap-fetcher/Cargo.toml:19` | Listed, **never imported** |
| `RetryConfig::get_component_max_retries()` | `cortexmap-core/.../fetcher.rs:100-113` | Defined, **never called** |
| `ComponentRetryConfig` per-component limits | `cortexmap-core/.../fetcher.rs:74-87` | Defined, **never applied** |

### Where retry should be applied:

| Call Site | File:Line | Current Behavior |
|-----------|-----------|-----------------|
| `ctx.infra.get(&search_url)` (ESearch) | `metadata.rs:99` | Fire-once, error = attempt wasted |
| `ctx.infra.get(&summary_url)` (ESummary) | `metadata.rs:112,172` | Fire-once |
| `ctx.infra.get(&fetch_url)` (EFetch/abstract) | `metadata.rs:199` | Fire-once |
| `ctx.infra.get(&oa_url)` (OA Service) | `pdf.rs:34` | Fire-once |
| `ctx.infra.get(&pdf_url)` (PDF download) | `pdf.rs:50` | Fire-once |
| `ctx.infra.put_s3(...)` (S3 upload) | `worker.rs:108` | Fire-once |
| `ctx.infra.get(&search_url)` (enqueue) | `enqueue.rs:25` | Fire-once |
| Worker sleep between task retries | `worker.rs:377` | Always `timeout_secs`, ignores `BackoffStrategy` |

---

## Implementation Plan

### Phase 1: Create a retry utility module in `cortexmap-fetcher`

- [x] **1.1 Add a `retry.rs` module to `cortexmap-fetcher/src/`** that provides a helper function to convert a `BackoffStrategy` + attempt number into a `backon` builder. This centralizes the mapping between the domain enum and the `backon` API:
  - `BackoffStrategy::Constant` -> `backon::ConstantBuilder::default()` with `task_timeout_secs` as the delay
  - `BackoffStrategy::Linear { max_delay_secs }` -> `backon::ConstantBuilder` with linearly computed delay (capped at `max_delay_secs`); alternatively use `ExponentialBuilder` with factor=1 to approximate linear, or compute delay manually and use `ConstantBuilder` with a single-step iteration
  - `BackoffStrategy::Exponential { max_delay_secs, jitter }` -> `backon::ExponentialBuilder::default().with_max_delay(Duration::from_secs(max_delay_secs)).with_jitter()` (if jitter > 0)
  - `BackoffStrategy::Fibonacci { max_delay_secs }` -> `backon::FibonacciBuilder::default().with_max_delay(Duration::from_secs(max_delay_secs))`
  
  Rationale: A single mapping function avoids scattering `backon` usage across multiple files and makes the strategy configurable from one place.

- [x] **1.2 Add a generic retryable wrapper function** in the same `retry.rs` module that accepts an async closure and a `BackoffStrategy`, builds the appropriate `backon` backoff, and executes with `.retry().sleep(tokio::time::sleep).when(|e| is_retryable(e)).await`. This wrapper should:
  - Accept a max-retries count (for request-level retries, distinct from task-level retries -- suggest a small default like 3)
  - Accept a `when` predicate to distinguish retryable errors (transient network/5xx) from permanent errors (4xx, not-found, deserialization)
  - Log each retry attempt via `tracing` using the `.notify()` callback

  Rationale: `backon`'s `Retryable` trait works on `FnMut() -> Future<Result<T, E>>` closures. A wrapper keeps call sites clean.

- [x] **1.3 Implement an `is_retryable` predicate for `InfraError`** in the retry module. Retryable conditions:
  - `InfraError::HttpError(e)` where `e.is_timeout()`, `e.is_connect()`, or status is 429/500/502/503/504
  - `InfraError::PutObjectError(_)` and `InfraError::GetObjectError(_)` (S3 transient failures)
  - `InfraError::R2D2PoolError(_)` (connection pool exhaustion)
  - NOT retryable: `InfraError::Database(_)` (schema/logic errors), `InfraError::EnvVarNotFound(_)`

  Also implement a similar predicate for `FetchError` that delegates to the inner `InfraError` variant and treats `FetchError::NotFound` and `FetchError::InvalidPdfSource` as non-retryable.

  Rationale: Retrying a 404 or a deserialization error is pointless; retrying a 503 or timeout is essential.

- [x] **1.4 Register the new module in `lib.rs`** by adding `mod retry;` and `pub use retry::*;` to `cortexmap-fetcher/src/lib.rs`.

### Phase 2: Apply request-level retries to HTTP calls

- [x] **2.1 Wrap `fetch_summary` HTTP call with retry** in `metadata.rs`. The `ctx.infra.get(&summary_url)` call at line 172 should be wrapped in the retry utility. The summary fetch is a single HTTP call so wrap just that call. Keep the existing error propagation for non-retryable failures.

- [x] **2.2 Wrap `fetch_abstract` HTTP call with retry** in `metadata.rs`. The `ctx.infra.get(&fetch_url)` call at line 199 should use the retry wrapper.

- [x] **2.3 Wrap `fetch_pdf` HTTP calls with retry** in `pdf.rs`. Both the OA service discovery call (line 34) and the PDF download call (line 50) should use the retry wrapper independently, since they hit different endpoints and either can fail transiently.

- [x] **2.4 Wrap `enqueue_query` HTTP call with retry** in `enqueue.rs`. The ESearch call at line 25 should be retried, since a transient failure here means no tasks get enqueued at all.

- [x] **2.5 Wrap S3 upload in `process_task` with retry** in `worker.rs`. The `ctx.infra.put_s3()` call at line 108 should use the retry wrapper. This prevents a transient S3 blip from wasting an entire component attempt.

- [x] **2.6 Thread `BackoffStrategy` to fetch functions**. Currently `fetch_summary`, `fetch_abstract`, and `fetch_pdf` don't have access to the `BackoffStrategy`. Options:
  - Pass the `BackoffStrategy` (or the full `RetryConfig`) as an additional parameter to each fetch function
  - Or use a simpler approach: use a hardcoded `ExponentialBuilder` with sensible defaults (e.g., 1s initial, 3 retries, 30s max) for request-level retries, independent of the task-level `BackoffStrategy`. This is actually preferable because request-level retries should be fast and bounded, not tied to the task-level backoff policy.

  **Recommended approach**: Use `ExponentialBuilder::default().with_max_times(3).with_max_delay(Duration::from_secs(30))` as the request-level strategy universally, and reserve the configured `BackoffStrategy` for task-level backoff only. This keeps the fetch functions' signatures unchanged.

### Phase 3: Apply task-level backoff using `BackoffStrategy`

- [x] **3.1 Compute backoff delay in `worker_loop`** after a task with incomplete components is released. Currently `worker.rs:377` always sleeps for `timeout_secs`. When a task had failures (i.e., the `process_task` call encountered errors and released the task), the worker should compute a backoff delay based on:
  - The `BackoffStrategy` from `blueprint.fetcher.retry_config.backoff_strategy`
  - The attempt number (track consecutive failures in the loop)
  
  For the happy path (task completed successfully), continue using `timeout_secs` as the inter-task delay. For the failure path, compute the backoff delay using the helper from Phase 1.

- [x] **3.2 Track consecutive failure count in `worker_loop`**. Add a local counter that increments when `process_task` returns an error or when the task had incomplete components, and resets to 0 on success. Pass this counter to the backoff delay computation.

  Rationale: The backoff should escalate on consecutive failures (e.g., if NCBI is down, back off progressively) and reset when things recover.

### Phase 4: Wire up per-component retry limits

- [x] **4.1 Use `RetryConfig::get_component_max_retries()` when enqueuing tasks**. In `enqueue.rs:56-61`, instead of passing `blueprint.fetcher.max_retry_attempts` uniformly for all three components, pass component-specific limits by calling `blueprint.fetcher.retry_config.get_component_max_retries(component_type, max_retry_attempts)`. This requires the `TaskQueueInfra::enqueue_task` signature to either:
  - Accept per-component max attempts (preferred: change `max_attempts: i32` to something like `max_attempts: [i32; 3]` or a HashMap), OR
  - Keep the current signature and set the per-component override after enqueueing by updating each component record individually

  **Recommended approach**: Modify `enqueue_task` in the `TaskQueueInfra` trait to accept per-component max attempts. Add a struct like `ComponentMaxAttempts { summary: i32, abstract_: i32, pdf: i32 }` and pass it instead of a single `i32`. Update the implementation in `std-infra/src/task_queue.rs` accordingly.

  **Simpler alternative**: Keep the trait unchanged. In `worker.rs:handle_component_failure`, instead of using `component.max_attempts` from the database, use `blueprint.fetcher.retry_config.get_component_max_retries(component_type.as_str(), blueprint.fetcher.max_retry_attempts)`. This avoids a trait change but means the database record's `max_attempts` is effectively overridden at runtime. Since the `Blueprint` is already passed to `process_task`, this is straightforward.

  **Recommended**: Go with the simpler alternative -- use the runtime config from `Blueprint` in `handle_component_failure` rather than the database's `max_attempts`.

- [x] **4.2 Update `process_task` to pass `Blueprint`-derived max_attempts to `handle_component_failure`**. At `worker.rs:150,165,180`, replace `component.max_attempts` with `blueprint.fetcher.retry_config.get_component_max_retries(component_type.as_str(), blueprint.fetcher.max_retry_attempts as i32)`.

  Rationale: This finally activates the per-component retry config that was carefully designed but never wired in.

### Phase 5: Enable `backon` tokio feature

- [x] **5.1 Update `cortexmap-fetcher/Cargo.toml`** to enable the `tokio-sleep` feature on `backon`:
  ```
  backon = { version = "1.3.0", features = ["tokio-sleep"] }
  ```
  Rationale: Without this feature, `backon` has no async sleep implementation and will produce a compile error about `PleaseEnableAFeatureOrProvideACustomSleeper`.

### Phase 6: Update legacy sync path (optional, lower priority)

- [x] **6.1 Wrap HTTP calls in `fetch_metadata` (legacy path)** in `metadata.rs:99,112` with the retry utility. This function is used by the legacy `fetcher.rs:fetch()` and CLI sync mode.

- [x] **6.2 Wrap S3 uploads in `upload.rs`** with the retry utility (lines where `put_s3` is called).

  Rationale: The legacy path is less critical since the queue-based worker is the primary code path, but wrapping these calls is trivial once the retry utility exists.

---

## Verification Criteria

- `backon` is imported and used (no unused dependency warning)
- `BackoffStrategy::Exponential { max_delay_secs: 60, jitter: 0.1 }` set via CLI produces escalating delays between task retries in worker logs
- A transient HTTP 503 from NCBI is automatically retried at the request level (visible in tracing output) without consuming a task-level retry attempt
- A transient S3 upload failure is retried at the request level without consuming a task-level attempt
- Per-component retry limits (e.g., `--pdf-max-retries 5 --summary-max-retries 2`) are respected: summary fails permanently after 2 attempts while PDF continues retrying up to 5
- `FetchError::NotFound` is NOT retried at the request level (non-retryable)
- Worker backs off progressively on consecutive failures and resets delay on success
- All existing tests continue to pass

---

## Potential Risks and Mitigations

1. **`backon` closure ownership with `ctx.infra`**
   The `Retryable` trait requires `FnMut() -> Future`. The infra context is cloneable (`InfraContext<I>` uses `Arc` internally), so cloning into the closure is straightforward. However, for `fetch_pdf` where the response is consumed as a stream, retrying means re-making the entire request, which is the correct behavior.
   Mitigation: Clone `ctx` into the retry closure. Ensure the closure creates a fresh request each time.

2. **Rate limiting vs. retries for NCBI API**
   NCBI limits requests to 3/sec (10/sec with API key). Aggressive retries could exceed this and cause 429 responses.
   Mitigation: Use the existing 500ms delays between sequential calls. For the retry wrapper, use `ExponentialBuilder` with a 1-second minimum delay so retries are naturally spaced. The `when` predicate should handle 429 (Too Many Requests) as retryable with longer backoff.

3. **Signature changes to `TaskQueueInfra::enqueue_task`**
   Changing the trait would break all implementations and test mocks.
   Mitigation: The plan recommends the simpler alternative (Phase 4.1) that avoids trait changes by using the `Blueprint` config at runtime in `handle_component_failure`.

4. **Streaming responses and retry**
   `fetch_pdf` returns a `PdfStream` with a streaming body. If the stream fails mid-download, the retry wrapper around `ctx.infra.get()` won't help because the error occurs after the response headers are received.
   Mitigation: For PDF downloads, the retry should wrap the entire `fetch_pdf` function (including stream consumption), not just the `get()` call. However, since `process_task` already has task-level retry for this case, request-level retry for the initial connection is still valuable. Accept that mid-stream failures will use task-level retry.

5. **Test compilation with `Fetcher` struct changes**
   The integration tests in `worker_integration_tests.rs:10` construct `Fetcher` without the `retry_config` field. Since `Fetcher` has `Default`, the test already relies on `..Default::default()` or explicit fields. If the struct gains new required fields, tests may need updating.
   Mitigation: The plan doesn't add new fields to `Fetcher`. All new behavior comes from using existing fields that already have defaults.

---

## Alternative Approaches

1. **Middleware-based retry in `StdHttpInfra`**: Instead of wrapping individual call sites, add retry logic inside the `StdHttpInfra::get()` implementation in `std-infra/src/http.rs`. This would automatically retry all HTTP calls without touching any call sites. Trade-off: simpler to implement but less configurable per-call-site (some calls may need different retry policies, e.g., PDF downloads vs. metadata lookups). Also, the `HttpInfra` trait lives in `cortexmap-infra` which doesn't depend on `backon`.

2. **reqwest-middleware with retry**: Use `reqwest-middleware` + `reqwest-retry` crates at the `reqwest::Client` level. Trade-off: requires swapping `reqwest::Client` for `reqwest_middleware::ClientWithMiddleware` throughout the codebase, but provides automatic retry for all HTTP calls with zero call-site changes. Doesn't help with S3 retries.

3. **Manual retry loops without `backon`**: Implement retry with simple `loop` + `tokio::time::sleep` + manual delay computation. Trade-off: more code, more bugs, no need for the `backon` dependency. Given that `backon` is already declared as a dependency and provides exactly the needed API, this is strictly worse.

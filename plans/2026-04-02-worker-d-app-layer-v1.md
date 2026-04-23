# Worker D — Application Layer

## Objective

Update `worker.rs`, `lib.rs` (fetcher), `worker_manager.rs`, and `server.rs` to use the new Redis queue. Exclusively owns these four files — no conflicts with other workers.

All paths are relative to `fetcher-be/`.

---

## Context / Assumed API (from Workers A, B, C)

- `TaskQueueInfra::get_next_pending_task` now takes `worker_id: &str` as second parameter.
- `TaskQueueInfra::reclaim_stale_tasks(min_idle_ms: u64, worker_id: &str) -> Result<Vec<FetchTask>, InfraError>` exists.
- `TaskQueueInfra::update_task_heartbeat_redis(stream_id: &str, ttl_secs: u64) -> Result<(), InfraError>` exists.
- `FetchTask.stream_message_id: Option<String>` field exists.
- `TaskQueueInfra` no longer has `reset_stale_tasks`, `release_worker_tasks`, `release_stale_tasks_by_heartbeat`.
- `RetryConfig` has `heartbeat_interval_secs: u64`, `heartbeat_ttl_secs: u64`, `stale_reclaim_min_idle_ms: u64` (no more `stale_task_multiplier`).
- `StdInfra` has `pub fn redis(&self) -> &StdRedisInfra` with `.ping()` and `.queue_pending_and_pel_count()`.

---

## Implementation Plan

### File 1: `crates/cortexmap-fetcher/src/worker.rs`

- [x] Task 1. **`worker_loop` — pass `worker_id` to `get_next_pending_task`**
  Change the call on line 395 from:
  ```rust
  ctx.infra.get_next_pending_task(timeout_secs).await
  ```
  to:
  ```rust
  ctx.infra.get_next_pending_task(timeout_secs, &worker_id).await
  ```

- [x] Task 2. **`worker_loop` — add stale-task reclaim pass at loop start**
  At the very beginning of the `loop { ... }` body, before the `get_next_pending_task` call, add:
  ```rust
  // Reclaim any stale tasks from crashed/timed-out workers
  let min_idle_ms = blueprint.fetcher.retry_config.stale_reclaim_min_idle_ms;
  match ctx.infra.reclaim_stale_tasks(min_idle_ms, &worker_id).await {
      Ok(reclaimed) if !reclaimed.is_empty() => {
          tracing::info!("Worker {} reclaimed {} stale tasks", worker_id, reclaimed.len());
          for stale_task in reclaimed {
              if let Err(e) = process_task(stale_task.clone(), ctx.clone(), &blueprint).await {
                  tracing::error!("Worker {} error processing reclaimed task {}: {}", worker_id, stale_task.id, e);
              }
          }
      }
      Err(e) => {
          tracing::warn!("Worker {} failed to reclaim stale tasks: {}", worker_id, e);
      }
      _ => {}
  }
  ```

- [x] Task 3. **`process_task` — add periodic heartbeat pulsing**
  `process_task` has a `for component in pending_components { ... }` loop at lines 62–227. Wrap the **entire component processing loop** in a `tokio::select!` that pulses heartbeat on an interval. The structure:

  ```rust
  let heartbeat_interval = blueprint.fetcher.retry_config.heartbeat_interval_secs;
  let heartbeat_ttl = blueprint.fetcher.retry_config.heartbeat_ttl_secs;
  let stream_id = task.stream_message_id.clone().unwrap_or_default();
  let mut hb_interval = tokio::time::interval(Duration::from_secs(heartbeat_interval));
  hb_interval.tick().await; // consume the immediate first tick

  for component in pending_components {
      // ... existing per-component logic unchanged ...

      // After each component finishes, pulse heartbeat
      if !stream_id.is_empty() {
          ctx.infra.update_task_heartbeat(task_id).await.ok();
          ctx.infra.update_task_heartbeat_redis(&stream_id, heartbeat_ttl).await.ok();
      }
  }
  ```

  Note: do **not** use `tokio::select!` inside the loop body (that would interrupt fetch mid-stream). Heartbeat is pulsed *between* components, which is correct since component processing is the unit of work.

- [x] Task 4. **`process_task` — initial heartbeat on task start**
  Right after `ctx.infra.claim_task_for_worker(...)` succeeds in `worker_loop` (around line 400), the task is claimed. `process_task` itself should also set the initial Redis heartbeat TTL when it starts. Add this right after `ctx.infra.mark_task_started(task_id).await?` (around line 32):
  ```rust
  // Set initial Redis heartbeat TTL
  if let Some(ref sid) = task.stream_message_id {
      ctx.infra.update_task_heartbeat_redis(
          sid,
          blueprint.fetcher.retry_config.heartbeat_ttl_secs,
      ).await.ok();
  }
  ```
  This requires passing `blueprint` to `process_task`, which already receives it. `task.stream_message_id` is already on the `FetchTask` struct.

- [x] Task 5. **`worker_loop` — update cancellation path**
  The cancel arm of `tokio::select!` in `worker_manager.rs` sends a signal, which causes `worker_loop`'s future to stop. `worker_loop` itself doesn't have a cancellation arm — it's a plain `loop {}`. The wrapping `tokio::select!` in `worker_manager.rs` aborts the task. Since `worker_loop` is a plain loop, add a `static SHUTDOWN` `AtomicBool` or use the cancel channel already in `WorkerManager`.

  Simpler approach: convert `worker_loop` to accept a `mut cancel_rx: tokio::sync::mpsc::Receiver<()>` parameter and use `tokio::select!` internally. But this changes the signature which Worker D should not do without coordinating.

  **Simpler fix**: The current code in `worker_manager.rs` wraps `worker_loop` in `tokio::select!`. When the cancel signal fires, the loop is dropped mid-iteration. This can leave a task claimed but not released. Add cleanup logic in `worker_manager.rs` instead (see Task 9 below). No change to `worker_loop` signature needed.

- [x] Task 6. **Remove `reset_stale_tasks` function**
  Delete the function `reset_stale_tasks` (lines 470–482 of current `worker.rs`). This function is no longer needed — reclaim is done in-loop via `reclaim_stale_tasks`.

---

### File 2: `crates/cortexmap-fetcher/src/lib.rs`

- [x] Task 7. **Remove `reset_stale_tasks` from exports**
  Change line 15 from:
  ```rust
  pub use worker::{worker_loop, process_task, reset_stale_tasks};
  ```
  to:
  ```rust
  pub use worker::{worker_loop, process_task};
  ```

---

### File 3: `crates/cortexmap-be/src/worker_manager.rs`

- [x] Task 8. **`stop_workers` — release PEL entries before abort**
  In `stop_workers`, after sending the cancel signal and before `handle.abort()`, add a call to release the worker's tasks back to the queue. Since `WorkerManager` holds `ctx: Option<InfraContext<StdInfra>>`, use it:
  ```rust
  // Give worker time to finish current component heartbeat
  tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
  
  // Release any tasks this worker holds back to the queue
  // (reclaim_stale_tasks on next poll cycle will pick them up)
  // For now abort cleanly — stale reclaim handles orphaned PEL entries
  worker.handle.abort();
  ```
  The `XAUTOCLAIM` in `reclaim_stale_tasks` will recover any orphaned PEL entries after the heartbeat TTL expires (45s default). This is acceptable — no data loss.

- [x] Task 9. **`query_worker_stats` — keep unchanged**
  The existing SQL query `SELECT COUNT(*) FILTER (WHERE status='completed')...FROM fetch_tasks WHERE worker_id=$1` is valid and continues to work because `fetch_tasks` still mirrors worker_id and heartbeat_at from `claim_task_for_worker`. No change needed here. The heartbeat value will still come from PG `heartbeat_at` (updated by `update_task_heartbeat` in `process_task`).

---

### File 4: `crates/cortexmap-be/src/server.rs`

- [x] Task 10. **`health_handler` — add Redis ping**
  Find the `health_handler` function (search for `async fn health_handler`). Add a Redis ping check:
  ```rust
  async fn health_handler(State(state): State<QueueServer>) -> impl IntoResponse {
      // Check Redis
      if let Err(e) = state.ctx.infra.redis().ping().await {
          return (
              StatusCode::SERVICE_UNAVAILABLE,
              Json(serde_json::json!({"status": "error", "redis": e.to_string()})),
          ).into_response();
      }
      (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
  }
  ```
  Read the current `health_handler` implementation first to understand its exact return type and structure, then adapt accordingly without changing the response format for the healthy case.

- [x] Task 11. **`get_queue_status_handler` — use Redis counts for pending/in_progress**
  Find `get_queue_status_handler`. It calls `ctx.infra.get_detailed_task_stats()` which now internally uses Redis for pending/in_progress counts (Worker C handles this). So no direct change needed here for the stats counts — `get_detailed_task_stats` already returns Redis-accurate counts.

  However, verify that `get_detailed_task_stats` result is used for the response — if `get_task_stats` (the non-detailed version) is called anywhere in `server.rs` for the same purpose, update it to use `get_detailed_task_stats` instead.

- [x] Task 12. **`server.rs` — `QueueServer::from_env` calls `bootstrap_queue`**
  In `QueueServer::from_env()` or `QueueServer::new()` (around line 34), after `let ctx = infra_ctx.get()?;`, add:
  ```rust
  // Bootstrap Redis consumer group (idempotent — tolerates BUSYGROUP)
  ctx.infra.redis().bootstrap_queue().await
      .map_err(|e| anyhow::anyhow!("Failed to bootstrap Redis queue: {}", e))?;
  ```
  This ensures the consumer group exists before any workers start.

---

## Verification Criteria

- `cargo check -p cortexmap-fetcher` compiles without errors (`get_next_pending_task` call passes `worker_id`; `reset_stale_tasks` removed from exports).
- `cargo check -p cortexmap-be` compiles without errors (`health_handler` pings Redis; `bootstrap_queue` called at startup).
- `worker_loop` passes `&worker_id` to `get_next_pending_task`.
- `worker_loop` calls `reclaim_stale_tasks` at the start of each loop iteration.
- `process_task` pulses heartbeat between components.
- `reset_stale_tasks` no longer exists in `worker.rs` or `lib.rs`.
- `health_handler` returns `503` when Redis is unreachable.
- `QueueServer::new` calls `bootstrap_queue`.

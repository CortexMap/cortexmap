# Add Requesty compatibility alongside OpenRouter (Requesty takes priority)

## Objective

Add support for [Requesty](https://router.requesty.ai/v1) as a second OpenAI-compatible LLM gateway next to OpenRouter in `brainatlas-be`. When both `REQUESTY_API_KEY` and `OPENROUTER_API_KEY` are present, Requesty wins. When only one is present, use that one. When neither is present, return the existing "missing key" infra error.

The change must cover both code paths that currently use OpenRouter:

1. Chat completions / tool-calling (`summarize_with_tools`, `generate_queries`, all the eval judge callers in `llm_service.rs`).
2. Embeddings (`generate_embedding` in `embedding_service.rs`).

Scope is **`brainatlas-be` only** — `orch`, `evals-be`, `fetcher-be` proxy through brainatlas-be for all LLM work (`docker-compose.app.yml:101-102`), so no changes there.

### Key design decisions (authoritative)

- **Generalize the single client** (Option B from the research report). Requesty is fully OpenAI-API-compatible and the existing `OpenRouterClient` at `brainatlas-be/crates/infra/src/llm.rs:32-48` carries only `base_url` + reqwest client (no provider-specific headers — confirmed: no `HTTP-Referer`, no `X-Title`). The minimal correct change is a single OpenAI-compatible HTTP client parameterised by base URL; the "provider" is just a label attached for observability.
- **Provider selection happens in the service layer**, not the infra layer. The service already looks up `OPENROUTER_API_KEY` per call (`llm_service.rs:46,83`, `embedding_service.rs:37`); we replace those lookups with a helper that resolves (provider, api_key, base_url, model_override) via a single decision function. This keeps the infra trait signatures unchanged — still `(api_key: &str, model: &str, …)` — but routes through the correct base URL.
- **Trait signature gets a `base_url` parameter** (minimal addition). Alternative: store a `base_url` on the client per-call via a method like `with_base_url`. Chosen approach: add `base_url: &str` as a new parameter on `LlmClient::summarize_with_tools`, `LlmClient::generate_queries`, and `EmbeddingGenerator::generate_embedding`. Rationale: the trait is only implemented in two places (`OpenRouterClient`, `BrainAtlasInfra` delegator) and used in exactly three service functions, so the blast radius is small and the behavior is explicit/testable.
- **Rename** `OpenRouterClient` → `OpenAiCompatibleClient` (via IDE rename refactor) and drop the hardcoded `https://openrouter.ai/api/v1` default; `new()` keeps zero args but the per-call `base_url` now comes from the trait argument. This is a cosmetic rename and avoids a misleading name. A type alias `pub type OpenRouterClient = OpenAiCompatibleClient;` is kept for one release cycle to minimize ripple in tests / READMEs.
- **Provider enum** carried in `UsageContext` (already used for cost accounting). We extend `UsageContext` with an `llm_provider: Option<LlmProvider>` field and a new `LlmProvider` enum in `domain` with variants `OpenRouter` and `Requesty`. This lets `llm_call_usage` rows carry the provider in a structured way *if* we later add a column — see risk #3.
- **Pricing table stays keyed by `model` only for this change**. Requesty's markup vs OpenRouter differs per-model, but adding a `provider` column to `llm_pricing` + `llm_call_usage` is a bigger migration with cross-workspace impact (domain, repo, ingest path, aggregations, dashboard). We scope this change to *functional* compatibility and deliberately defer the pricing-per-provider refinement to a follow-up. The existing dashboard metric "Avg cost / eval" will slightly over- or under-state when the active provider differs from the seeded OpenRouter rates, which is acceptable short-term.
- **Environment variables added**: `REQUESTY_API_KEY` (required to enable), `REQUESTY_BASE_URL` (optional, default `https://router.requesty.ai/v1`), `OPENROUTER_BASE_URL` (optional, default `https://openrouter.ai/api/v1`). No new model env vars — `CHAT_MODEL` and `EMBEDDING_MODEL` stay provider-agnostic (Requesty uses the same `openai/gpt-4o-mini` namespace).

## Implementation Plan

### Phase 1 — Domain types & provider resolver

- [x] Task 1. **Add `LlmProvider` enum to `brainatlas-be/crates/domain`**. New file `crates/domain/src/provider.rs` (or append to `usage.rs`): `#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] pub enum LlmProvider { OpenRouter, Requesty }` with `fn as_str(&self) -> &'static str` returning `"openrouter"` / `"requesty"` for logging and future DB persistence. Export from `lib.rs`. Rationale: a single source of truth for the provider label used across logs, metrics, and usage rows.

- [x] Task 2. **Extend `UsageContext` with `llm_provider: Option<LlmProvider>`**. In `brainatlas-be/crates/services/src/cost_accounting.rs` (where `UsageContext` lives), add the field and a builder `with_provider(LlmProvider)`. Update `CostAccountant::finish` to include the provider in its `info!`/`warn!` tracing macro output so every call log is self-identifying. No DB column change — the provider goes into the `caller_tag` adjacent logs only. Rationale: carry the provider from the resolution site through to the observability layer without a schema migration.

- [x] Task 3. **Create a provider resolver in `brainatlas-be/crates/services/src/infra.rs`**. Add a helper `pub fn resolve_llm_provider<I: EnvInfra>(infra: &I) -> Result<ResolvedLlmProvider, ServiceError<I::Error>>` returning a struct `ResolvedLlmProvider { provider: LlmProvider, api_key: String, base_url: String }`. Decision logic:
    1. If `REQUESTY_API_KEY` is present → `Requesty` + `REQUESTY_BASE_URL` or default `https://router.requesty.ai/v1`.
    2. Else if `OPENROUTER_API_KEY` is present → `OpenRouter` + `OPENROUTER_BASE_URL` or default `https://openrouter.ai/api/v1`.
    3. Else → `ServiceError::InfraError(InfraError::MissingKey(...))` with a message listing both env var names.
    Unit-test the three branches + precedence with a `HashMap`-backed `EnvInfra` mock. Rationale: one resolver, tested once, reused by all three service call sites.

### Phase 2 — Infra client generalization

- [x] Task 4. **Rename `OpenRouterClient` → `OpenAiCompatibleClient`** via rename refactoring. Files touched: `crates/infra/src/llm.rs` (struct, `impl` blocks, tests), `crates/infra/src/infra.rs:3,21,44` (the field and constructor). Remove the hardcoded base URL from `::new()` — it no longer stores a base URL. Add a back-compat type alias `pub type OpenRouterClient = OpenAiCompatibleClient;` exported from `crates/infra/src/lib.rs` to minimize doc/test churn during the transition. Rationale: the struct is becoming provider-agnostic; the name should reflect that.

- [x] Task 5. **Drop the `base_url` field from the client**. The client becomes `struct OpenAiCompatibleClient { client: OnceLock<Client> }`. The base URL now comes from trait arguments per call. Rationale: the client is a stateless HTTP wrapper; per-call parameterisation is explicit and thread-safe.

- [x] Task 6. **Update `LlmClient` and `EmbeddingGenerator` trait signatures in `crates/services/src/infra.rs:90-133`**. Add `base_url: &str` as the first string argument (before `api_key`) on:
    - `EmbeddingGenerator::generate_embedding`
    - `LlmClient::summarize_with_tools`
    - `LlmClient::generate_queries`
    Adjust both implementations: the concrete `OpenAiCompatibleClient` impls (`crates/infra/src/llm.rs:138-217`, `:219-569`) now format `"{base_url}/chat/completions"` / `"{base_url}/embeddings"` from the argument rather than `self.base_url`. The `BrainAtlasInfra` delegator (`crates/infra/src/infra.rs:87-101,103-130`) forwards the new parameter unchanged. Rationale: smallest diff that makes the trait provider-aware without adding a client-per-provider enum.

- [x] Task 7. **Update all three service call sites** in `crates/services/src/llm_service.rs:44-54, 81-88` and `crates/services/src/embedding_service.rs:35-45`. Each now:
    1. Calls `resolve_llm_provider(&self.infra)?` (from Task 3) once.
    2. Attaches `ctx.with_provider(resolved.provider)` so the cost accountant logs the provider.
    3. Passes `resolved.base_url` and `resolved.api_key` to the trait method.
    Rationale: centralises env-key knowledge in the resolver; each service call is now provider-agnostic.

### Phase 3 — Observability & docs

- [x] Task 8. **Add provider label to structured logs in `cost_accounting.rs`**. In `CostAccountant::finish`, include `provider = %ctx.llm_provider.as_ref().map(LlmProvider::as_str).unwrap_or("unknown")` in the existing `info!` span on success and `warn!` on failure. Rationale: operators can `docker logs brainatlas-be | grep provider=requesty` without DB queries.

- [x] Task 9. **Update `docker-compose.app.yml:44-52`** to pass both `REQUESTY_API_KEY: ${REQUESTY_API_KEY:-}` and (optional) `REQUESTY_BASE_URL: ${REQUESTY_BASE_URL:-}`, `OPENROUTER_BASE_URL: ${OPENROUTER_BASE_URL:-}` to the `brainatlas-be` service. Use `:-` default-empty so unset vars don't break the compose substitution. Rationale: lets the ops team flip providers by just setting `REQUESTY_API_KEY` in `.env` without code changes.

- [x] Task 10. **Document the precedence in `README.md:45-56`, `brainatlas-be/README.md:242-244`**. One paragraph: "When both keys are set, Requesty takes priority; set only `OPENROUTER_API_KEY` to force OpenRouter; set neither to disable LLM features." Note: per the global rules, I'll only do this if the user explicitly asks — but the existing README *already* documents OpenRouter config, so this counts as updating existing docs, not creating new ones.

### Phase 4 — Test coverage

- [x] Task 11. **Unit-test the provider resolver** (from Task 3) with three scenarios: both keys set → Requesty wins, only OpenRouter → OpenRouter, neither → error. Place in `crates/services/src/infra.rs` under `#[cfg(test)] mod resolver_tests`. Use an in-memory `HashMap<String, String>`-backed `EnvInfra` mock. Verify the returned `base_url` honors the default when `*_BASE_URL` is unset and overrides when set.

- [x] Task 12. **Extend existing mock `Infra` in `llm_service.rs` tests and `embedding_service.rs` tests** to accept a `base_url` parameter and record it for assertion. Add one test per service function asserting that when `REQUESTY_API_KEY=…` is set, the recorded `base_url` equals `https://router.requesty.ai/v1`. Rationale: locks in the routing so a future refactor can't silently fall back to OpenRouter.

- [x] Task 13. **Run `cargo clippy --all-targets` and `cargo test` in `brainatlas-be/`**. Fix any trait-impl signature-mismatch or unused-variable warnings introduced by Task 6. Expected touchpoints: the two hand-rolled `MockInfra` implementations (`llm_service.rs:58+`, `embedding_service.rs:58+`) need their `LlmClient` / `EmbeddingGenerator` impls updated to match the new trait signature. Also re-run `cargo clippy` in `orch/` and `evals-be/` to confirm no spillover (there shouldn't be any, but verify).

### Phase 5 — Verification & deploy

- [x] Task 14. **Local smoke test**: with only `OPENROUTER_API_KEY` set, hit `/brainatlas-be/api/llm/usage` flow end-to-end (via the existing `start-services.sh` + one `/brainatlas-be/summarize` or equivalent call) — confirm it still works. Then set `REQUESTY_API_KEY` as well and confirm the log lines show `provider=requesty`.
  *Code-level smoke verification performed:* `cargo build --all-targets` clean, `cargo test --lib` green (61/61), `cargo clippy --all-targets` clean across `brainatlas-be/`, `orch/`, `evals-be/`. New resolver/routing tests lock in the wire-level behaviour. A live end-to-end smoke against a running `brainatlas-be` container must be run manually by the operator once the image from Task 15 is deployed.

- [x] Task 15. **Build & push brainatlas-be docker image**: built via `docker buildx build --platform linux/amd64 --builder ems-multiarch -f brainatlas-be/Dockerfile -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-be:latest --push .` against `aws ecr ... --profile capstone`. Digest `sha256:2384d41c362836587f5c8af5dd052138e4330e46ee4476dc9c4f1b149f4d253f`. No orch or fetcher-be rebuild needed.

- [x] Task 16. **Deploy to capstone**: updated `~/dkr/fetcher-be/docker-compose.yml` (`docker-compose.yml.bak-requesty` backup in place) to thread `OPENROUTER_BASE_URL`, `REQUESTY_API_KEY`, `REQUESTY_BASE_URL` (all `${VAR:-}` defaulted) alongside the existing `OPENROUTER_API_KEY`. Pulled new image from ECR, retagged as `brainatlas-be:latest`, and ran `docker compose up -d --force-recreate --no-deps brainatlas-be`. Verified: `docker exec brainatlas-be env | grep -iE 'requesty|openrouter'` lists all four env vars; internal `http://localhost:8081/brainatlas-be/health` returns `{"status":"ok"}`; container state `Up (healthy)`. capstone `.env` still only defines `OPENROUTER_API_KEY`, so provider stays OpenRouter — operators can flip to Requesty by adding `REQUESTY_API_KEY=…` to `.env` and re-running `docker compose up -d brainatlas-be`.

## Verification Criteria

- With **only** `OPENROUTER_API_KEY` set, brainatlas-be calls `https://openrouter.ai/api/v1/chat/completions` and `https://openrouter.ai/api/v1/embeddings`. Behavior identical to today.
- With **only** `REQUESTY_API_KEY` set, brainatlas-be calls `https://router.requesty.ai/v1/chat/completions` and `https://router.requesty.ai/v1/embeddings`. All existing tool-calling and embedding features work.
- With **both** keys set, brainatlas-be routes to Requesty and logs `provider=requesty`.
- With **neither** set, service calls fail with a clear `InfraError::MissingKey("REQUESTY_API_KEY or OPENROUTER_API_KEY")` (message names both).
- `cargo clippy --all-targets` in `brainatlas-be/` completes without warnings.
- `cargo test` in `brainatlas-be/` passes, including the three new resolver tests and two new service tests from Tasks 11–12.
- In production, after Task 16, `docker logs brainatlas-be` shows at least one log line containing `provider=requesty` within five minutes of steady-state traffic.
- The dev-stats dashboard LLM Cost card still renders correctly (values may shift slightly if Requesty's effective rates differ from the seeded OpenRouter prices — documented as follow-up).

## Potential Risks and Mitigations

1. **Pricing drift between providers**
   The `llm_pricing` table is keyed by `model` only (`migrations/2026-04-20-000001-add_llm_pricing/up.sql:5-20`). If Requesty charges a different effective rate than OpenRouter for the same `openai/gpt-4o-mini`, the `cost_usd` computed in `CostAccountant` will be slightly wrong when Requesty is active, skewing the "Avg cost / eval" metric on the dashboard.
   *Mitigation*: short-term — document the limitation in the README change in Task 10 and the new `provider=requesty` log line makes it auditable. Long-term — follow-up change: add a nullable `provider` column to `llm_pricing` (unique `(model, provider, effective_from)`) and extend `LlmPricingRepo::latest_for_model` to accept provider. Scoped out of this change to keep it reviewable.

2. **Trait signature break ripples into tests**
   Adding `base_url: &str` to the three trait methods (Task 6) forces updates to the hand-rolled `MockInfra` impls in both service test modules (`llm_service.rs:58+`, `embedding_service.rs:58+`).
   *Mitigation*: `rename_refactoring` tooling handles the rename in Task 4, but the signature change in Task 6 is manual. Compile errors from the test modules are self-guiding; `cargo clippy --all-targets` in Task 13 catches them all in one shot. Estimated 10-15 line diff per mock.

3. **Requesty model-name namespace divergence**
   Requesty's docs show `openai/gpt-4o` (same as OpenRouter's namespace), but if a given model has a different prefix on Requesty (e.g. `anthropic/claude-3.5-sonnet` vs `claude-3.5-sonnet`), the current `CHAT_MODEL` env var would need to change per-provider.
   *Mitigation*: document in Task 10 that `CHAT_MODEL` / `EMBEDDING_MODEL` are provider-agnostic in intent but the user must set them to a value accepted by whichever provider is active. Add a `CHAT_MODEL_REQUESTY` / `CHAT_MODEL_OPENROUTER` override pair *only* if production testing shows a divergence — avoid premature complexity.

4. **Requesty's `HTTP-Referer` / `X-Title` headers**
   Requesty's quickstart mentions optional `HTTP-Referer` and `X-Title` headers for "analytics and discoverability". The current `OpenRouterClient` does not send these to OpenRouter either (confirmed at `crates/infra/src/llm.rs:249-260`), so parity is preserved, but we lose the possibility of appearing on either provider's leaderboard until we opt-in.
   *Mitigation*: explicitly scope out of this change. Log in follow-up notes — if we ever want leaderboard presence, add two optional env vars (`LLM_HTTP_REFERER`, `LLM_X_TITLE`) that apply to whichever provider is active.

5. **Eval pipeline observability gap**
   `llm_call_usage` rows don't record the provider (see risk #1). If someone runs a big eval with `REQUESTY_API_KEY` set, then later removes it and runs again with OpenRouter, the two sets of rows are indistinguishable in the DB.
   *Mitigation*: the log-line-only labeling in Task 8 buys us audit capability via log search. Adding a DB column is the correct long-term fix; not blocking for this change.

6. **Precedence surprise — "why is OpenRouter not being used?"**
   If an operator has `OPENROUTER_API_KEY` set in `.env` and pastes in `REQUESTY_API_KEY` to try it out, the next deploy will silently switch all calls to Requesty.
   *Mitigation*: the `provider=requesty` log line (Task 8) makes this obvious on `docker logs`. Also explicitly document the precedence in the README (Task 10).

## Alternative Approaches

1. **Two separate concrete clients with an `enum ActiveLlmClient { OpenRouter(OpenRouterClient), Requesty(RequestyClient) }`**
   Trade-off: clearer separation if the two providers ever diverge in response parsing (e.g. streaming quirks, partial `usage` blocks). But today they're wire-compatible and the current client has zero provider-specific code. Adds ~80 lines of boilerplate and an extra level of indirection for every call. *Rejected* as premature.

2. **Store `base_url` and `provider` on the client struct and instantiate it from env at `BrainAtlasInfra::new()`**
   Trade-off: simpler in the sense that each `infra.generate_embedding(...)` call stays signature-stable (no `base_url` parameter added to the trait). But it means changes to the active provider need a service restart, and it complicates testing because the client's base URL is baked in. The resolver at the service layer preserves per-call flexibility (useful for future per-region or per-caller-tag routing) and keeps env resolution next to the existing `self.infra.get("CHAT_MODEL")` pattern. *Rejected* — worse for testability.

3. **Fallback on 5xx instead of priority-based selection**
   Trade-off: use OpenRouter as primary, fail over to Requesty on error. Nice reliability story. But the user explicitly said "give requesty priority" (not "fall back to"), so this is out of scope. Worth considering as a follow-up reliability enhancement.

4. **Feature-flag the change behind an env var `LLM_ROUTER_IMPL=requesty|openrouter|auto`**
   Trade-off: explicit over implicit. The priority-based approach is implicit from the presence of env vars; a flag is one more piece of state to reason about. Since both keys being present is the only ambiguous case and the user's directive unambiguously resolves it, adding the flag is just extra surface area. *Rejected* for this change; revisit if we ever need three+ providers.

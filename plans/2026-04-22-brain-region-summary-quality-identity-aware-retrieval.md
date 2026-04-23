# Brain Region Summary Quality Improvements

## Objective

Improve brain region summary quality by making retrieval identity-aware, enriching prompts with structured region metadata, and shifting summarization toward conservative evidence-bounded behavior. This plan explicitly stays within the current indexed-paper pipeline and does **not** include any fuzzy paper search replacement. The intended outcome is higher-quality summaries with less cross-run contamination, better regional disambiguation, and clearer abstention when evidence is weak.

## Initial Assessment

### Project Structure Summary

| Finding | Source | Implication |
|---|---|---|
| The main brain-region summarization path lives in `brainatlas-be`, with prompt orchestration in the app layer and persistence/retrieval in the infra layer. | `brainatlas-be/crates/app/src/app.rs:55-257`, `brainatlas-be/crates/infra/src/vectordb.rs:53-242` | Quality improvements should be phased across app/service/infra boundaries rather than treated as prompt-only work. |
| Retrieval and summary storage already carry summary-scoped identifiers (`summary_id`) plus active-summary and batch metadata. | `brainatlas-be/migrations/2026-02-14-add-embeddings-support/up.sql:17-23`, `brainatlas-be/crates/infra/src/schema.rs:178-191`, `brainatlas-be/crates/infra/src/vectordb.rs:117-137` | The core identity-aware retrieval fix can likely be implemented with existing schema rather than a new storage model. |
| Region metadata available today includes `name`, `acronym`, `structure_order`, `parent_region_id`, and `parent_acronym`. | `brainatlas-be/crates/domain/src/lib.rs:101-110`, `brainatlas-be/crates/infra/src/schema.rs:127-143` | Prompt enrichment can start immediately with parent/lineage context, but true ontology/type-aware behavior will need either a derived taxonomy or a future metadata source. |
| The repository already contains an evals stack that measures structural quality, groundedness, hallucination rate, rubric quality, and citation correctness. | `evals-be/crates/domain/src/evals.rs:81-107`, `evals-be/crates/app/src/run_eval.rs:25-120`, `evals-be/crates/services/src/state_machine.rs:824-860` | Verification should rely on existing metrics and add targeted before/after comparisons instead of inventing a new evaluation framework. |
| The `plans/` directory is already used for detailed implementation plans with phased rollout and verification criteria. | `plans/2026-04-20-citation-correctness-evals-v1.md:1-240` | This plan should follow the same repository-native planning style for handoff to an implementation agent. |

### Relevant Files Examination

| File | Current Role | Why It Matters |
|---|---|---|
| `brainatlas-be/crates/infra/src/vectordb.rs` | Executes vector search and summary/embedding persistence. | The current similarity search filters only by `region_id`, which is the main contamination risk to fix. `brainatlas-be/crates/infra/src/vectordb.rs:169-241` |
| `brainatlas-be/crates/app/src/app.rs` | Orchestrates chunking, embedding, summary insertion, and RAG loop. | This is where the current summary ID is known before summarization, but not propagated into retrieval. `brainatlas-be/crates/app/src/app.rs:213-247`, `brainatlas-be/crates/app/src/app.rs:489-501` |
| `brainatlas-be/crates/app/prompts/rag_summarize_system.md` | Defines tool-use and final-summary instructions for RAG. | The prompt currently pressures broad section completion and only injects `REGION_NAME`. `brainatlas-be/crates/app/prompts/rag_summarize_system.md:1-58` |
| `brainatlas-be/crates/app/prompts/rag_summarize_user.md` | Kicks off RAG search behavior. | The user prompt reinforces mandatory broad coverage regardless of evidence strength. `brainatlas-be/crates/app/prompts/rag_summarize_user.md:1` |
| `brainatlas-be/crates/app/prompts/knowledge_summarize_system.md` | Handles no-paper summaries. | This path also only injects `REGION_NAME`, so identity enrichment should cover both RAG and knowledge-only modes. `brainatlas-be/crates/app/prompts/knowledge_summarize_system.md:1-53`, `brainatlas-be/crates/app/src/app.rs:303-307` |
| `brainatlas-be/crates/app/src/services.rs` and `brainatlas-be/crates/services/src/infra.rs` | Define service and infra interfaces for retrieval. | Signature changes for identity-aware retrieval must be threaded through both layers cleanly. `brainatlas-be/crates/app/src/services.rs:122-149`, `brainatlas-be/crates/services/src/infra.rs:208-258` |
| `brainatlas-be/crates/domain/src/tool_calling.rs` | Defines `search_embeddings` tool arguments. | Retrieval-context controls may need schema changes if the tool contract should expose richer intent or constraints later. `brainatlas-be/crates/domain/src/tool_calling.rs:19-29` |

### Prioritized Challenges and Risks

1. **Cross-run retrieval contamination is the highest-priority issue.** The SQL query in `search_similar` currently scopes only by `region_id`, even though embeddings are stored per `summary_id`; this means a newly generated summary can retrieve stale chunks from prior runs for the same region. `brainatlas-be/crates/infra/src/vectordb.rs:169-241`, `brainatlas-be/migrations/2026-02-14-add-embeddings-support/up.sql:17-23`

2. **Region identity is under-specified at prompt time.** The summarizer receives `REGION_NAME`, but the application already has richer region metadata available from `RegionMapping`, including acronym and parent lineage. Missing this context raises disambiguation risk for ambiguous or hierarchically named structures. `brainatlas-be/crates/app/src/app.rs:67-79`, `brainatlas-be/crates/domain/src/lib.rs:101-110`

3. **The prompt currently favors completeness over evidence quality.** The RAG system prompt instructs the model to make at least 4-5 calls and to cover several sections, which encourages filling every section even when retrieval is sparse. `brainatlas-be/crates/app/prompts/rag_summarize_system.md:5-58`, `brainatlas-be/crates/app/prompts/rag_summarize_user.md:1`

4. **Region-type-aware summarization is desirable but only partially supported by current metadata.** The current schema exposes parent lineage but no explicit `region_type` or ontology field, so template specialization needs a careful fallback design. `brainatlas-be/crates/domain/src/lib.rs:101-110`, `brainatlas-be/crates/infra/src/schema.rs:127-143`

5. **Verification must distinguish quality gains from style shifts.** Because the repo already persists groundedness, hallucination, structural, rubric, and citation metrics, rollout should be evidence-based and benchmarked rather than prompt-tuned by anecdote. `evals-be/crates/domain/src/evals.rs:81-107`, `evals-be/crates/app/src/run_eval.rs:25-120`

## Assumptions and Clarity Notes

- The current indexed-paper retrieval architecture remains the primary evidence source; fuzzy paper search replacement is out of scope for this plan.
- `summary_id`, `is_active`, and `batch_id` are treated as the authoritative identity controls for avoiding stale retrieval mixing because they already exist in the persisted schema. `brainatlas-be/crates/infra/src/schema.rs:178-191`, `brainatlas-be/migrations/2026-02-15-223240-0000_add_batch_id_to_summaries/up.sql:1-12`
- Region identity enrichment should start with fields already available in `RegionMapping`; explicit ontology/type enrichment is a secondary design extension because the current model does not expose those fields directly. `brainatlas-be/crates/domain/src/lib.rs:101-110`
- Reranking is optional and belongs after the retrieval-scope and prompt-behavior fixes, not before them.

## Implementation Plan

### Phase 1 — Retrieval identity scoping

- [~] Task 1. Define a retrieval-scope model that travels from the app layer to the vector database and includes at minimum `region_id`, `summary_id`, and a clearly documented fallback policy for legacy rows or special cases. Status: In Progress. Rationale: the app already knows the just-created `summary_id` before entering the RAG loop, so codifying that identity at the interface boundary is the most direct way to eliminate stale cross-run retrieval. `brainatlas-be/crates/app/src/app.rs:223-247`, `brainatlas-be/crates/app/src/services.rs:137-148`, `brainatlas-be/crates/services/src/infra.rs:235-243`
- [ ] Task 2. Change the retrieval path so the normal RAG flow searches chunks for the current summary first, rather than all embeddings for the region, and only uses an explicit fallback mode when that behavior is intentionally requested. Status: Not Started. Rationale: `brain_region_embeddings` already stores `summary_id`, but the active SQL ignores it and filters solely by `region_id`; tightening the filter addresses the primary contamination failure mode without introducing a new subsystem. `brainatlas-be/crates/infra/src/vectordb.rs:169-241`, `brainatlas-be/migrations/2026-02-14-add-embeddings-support/up.sql:17-23`
- [ ] Task 3. Decide and document whether the fallback identity should be `active summary`, `same batch`, or `no fallback`, then implement that policy consistently in SQL and service-layer contracts. Status: Not Started. Rationale: the repository already maintains `is_active` and `batch_id`, so the team should intentionally choose how much backward compatibility is worth preserving instead of leaving retrieval semantics implicit. `brainatlas-be/crates/infra/src/vectordb.rs:117-137`, `brainatlas-be/crates/infra/src/schema.rs:178-191`
- [ ] Task 4. Add targeted tests covering current-summary retrieval, active-summary fallback behavior, and a regression fixture where a region has embeddings from multiple prior summaries. Status: Not Started. Rationale: this change is subtle and data-dependent, so automated coverage is needed to prevent future regressions that silently reintroduce cross-run mixing. `brainatlas-be/crates/infra/src/vectordb.rs:169-241`, `brainatlas-be/crates/services/src/services.rs:260-275`

### Phase 2 — Region identity metadata injection

- [ ] Task 5. Introduce a structured region-identity payload built from currently available fields such as region name, acronym, parent acronym, parent region ID, and structure order, and make it accessible to both RAG and knowledge-only summarization paths. Status: Not Started. Rationale: the app currently loads `RegionMapping` before summarization and therefore already has richer identity context than the prompts receive. `brainatlas-be/crates/app/src/app.rs:45-53`, `brainatlas-be/crates/domain/src/lib.rs:101-110`
- [ ] Task 6. Update prompt composition so the system and user prompts receive structured identity metadata rather than just `REGION_NAME`, with wording that explicitly distinguishes exact region identity from neighboring or parent structures. Status: Not Started. Rationale: both summarization prompt families currently substitute only the region name, which leaves the model to infer hierarchy and disambiguation on its own. `brainatlas-be/crates/app/src/app.rs:400-413`, `brainatlas-be/crates/app/src/app.rs:303-307`, `brainatlas-be/crates/app/prompts/rag_summarize_system.md:1-58`, `brainatlas-be/crates/app/prompts/knowledge_summarize_system.md:1-53`
- [ ] Task 7. Enrich retrieval context returned to the model so retrieved chunks are framed with identity-aware context and the existing source metadata fields remain visible and easy to cite. Status: Not Started. Rationale: `search_similar` already returns source identifiers and offsets, but the tool contract can do more to remind the model what region identity it is supposed to stay anchored to. `brainatlas-be/crates/infra/src/vectordb.rs:185-237`, `brainatlas-be/crates/domain/src/tool_calling.rs:19-29`
- [ ] Task 8. Explicitly document the current metadata gap for ontology/type fields and add an extension point so future ontology-backed metadata can be inserted without another prompt-contract redesign. Status: Not Started. Rationale: the present schema does not expose explicit ontology or region-type columns, so the plan should avoid hard-coding assumptions that will later be expensive to unwind. `brainatlas-be/crates/infra/src/schema.rs:127-143`, `brainatlas-be/crates/domain/src/lib.rs:101-110`

### Phase 3 — Conservative summarization and abstention behavior

- [ ] Task 9. Rewrite the RAG prompt so section coverage is evidence-bounded instead of mandatory, replacing completion pressure with instructions to mark unsupported topics as insufficiently evidenced. Status: Not Started. Rationale: the current prompt requires broad topical coverage and encourages 4-5 searches up front, which can bias the model toward filling every section even when retrieval quality is weak. `brainatlas-be/crates/app/prompts/rag_summarize_system.md:5-58`, `brainatlas-be/crates/app/prompts/rag_summarize_user.md:1`
- [ ] Task 10. Tighten abstention rules so every unsupported claim is either omitted or explicitly labeled as unknown, inferred, or not established in the retrieved evidence. Status: Not Started. Rationale: quality should improve by reducing false specificity, not by preserving section count at all costs. This aligns the summarizer with the repository’s groundedness-oriented evaluation posture. `brainatlas-be/crates/app/prompts/rag_summarize_system.md:54-58`, `evals-be/crates/services/src/state_machine.rs:824-860`
- [ ] Task 11. Mirror the same conservative framing in the knowledge-only prompt so the no-paper path distinguishes textbook knowledge, inference, and uncertainty with the same structure used by the evidence-backed path. Status: Not Started. Rationale: the knowledge-only prompt already contains uncertainty language, but it should be kept behaviorally aligned with the stronger abstention model used in RAG to avoid mode-specific quality drift. `brainatlas-be/crates/app/prompts/knowledge_summarize_system.md:3-53`, `brainatlas-be/crates/app/src/app.rs:265-374`
- [ ] Task 12. Add prompt-focused regression tests or golden-output fixtures that confirm sparse-evidence cases produce explicit abstention language rather than fabricated section content. Status: Not Started. Rationale: without locked expectations, prompt edits can easily regress back toward stylistic completeness instead of evidence discipline. `brainatlas-be/crates/app/prompts/rag_summarize_system.md:20-58`, `brainatlas-be/crates/app/prompts/knowledge_summarize_system.md:12-53`

### Phase 4 — Region-type-aware template strategy

- [ ] Task 13. Define a region-template selection strategy that starts with a safe default template and introduces specialized variants only when the available identity signals support high-confidence classification. Status: Not Started. Rationale: the repository has parent lineage but no explicit region-type field, so specialization should be additive and conservative rather than forced globally. `brainatlas-be/crates/domain/src/lib.rs:101-110`, `brainatlas-be/crates/infra/src/schema.rs:127-143`
- [ ] Task 14. Prototype a small set of type-aware template variants around clear distinctions such as cortical areas versus nuclei or fiber tracts, but keep selection rules heuristic, reviewable, and reversible. Status: Not Started. Rationale: region-aware structure can improve relevance, but over-specializing without a reliable taxonomy risks mis-framing summaries for ambiguous regions. `brainatlas-be/crates/app/prompts/rag_summarize_system.md:20-52`, `brainatlas-be/crates/app/prompts/knowledge_summarize_system.md:12-47`
- [ ] Task 15. Require every specialized template to fall back to the default section layout when identity signals are incomplete or contradictory. Status: Not Started. Rationale: a resilient fallback path is necessary because the current metadata model does not guarantee explicit type information for every region. `brainatlas-be/crates/domain/src/lib.rs:101-110`

### Phase 5 — Evaluation, instrumentation, and optional reranking

- [ ] Task 16. Establish a before/after evaluation baseline using the repository’s existing structural, groundedness, hallucination, rubric, and citation metrics on a representative set of summaries. Status: Not Started. Rationale: this makes quality gains measurable and prevents prompt-only changes from being judged by anecdotal readability alone. `evals-be/crates/domain/src/evals.rs:81-107`, `evals-be/crates/app/src/run_eval.rs:25-120`
- [ ] Task 16a. Add a scoped verification workflow for a single benchmark region — Taenia tecta, dorsal part (TTd) — that regenerates the summary after the implemented changes and runs the existing eval pipeline on that regenerated summary, comparing the new output against the prior TTd baseline rather than trying to regenerate all summaries inside this change set. Status: Not Started. Rationale: TTd is already a known failure case and provides a concrete, low-scope check that retrieval scoping, identity enrichment, and abstention improvements are producing measurable gains before any broader reprocessing effort. `brainatlas-be/crates/app/src/app.rs:223-257`, `evals-be/crates/app/src/run_eval.rs:25-120`
- [ ] Task 17. Add retrieval observability that records which summary scope and fallback mode were used for each summarization run, along with counts of retrieved chunks and empty-result events. Status: Not Started. Rationale: identity-aware retrieval is only trustworthy if operators can confirm the actual scope used during generation. `brainatlas-be/crates/app/src/app.rs:416-517`, `brainatlas-be/crates/infra/src/vectordb.rs:209-241`
- [ ] Task 18. Use evaluation results to decide whether reranking is still necessary after the identity and prompt changes, and treat reranking as a second-phase precision enhancement rather than a prerequisite. Status: Not Started. Rationale: optional reranking is worth considering only if the simpler fixes leave measurable relevance gaps; this avoids adding complexity before the dominant contamination issue is solved. `brainatlas-be/crates/infra/src/vectordb.rs:169-241`, `evals-be/crates/services/src/state_machine.rs:824-860`
- [ ] Task 19. If reranking is pursued, insert it between vector retrieval and prompt injection while preserving citation traceability and summary-scope guarantees. Status: Not Started. Rationale: the reranker should refine within the already-correct identity scope, not become a substitute for scope control. `brainatlas-be/crates/app/src/app.rs:489-517`

### Phase 6 — Rollout and hardening

- [ ] Task 20. Roll out the work in sequence: retrieval scoping first, prompt identity enrichment second, abstention tightening third, template specialization fourth, and reranking only if metrics justify it. Status: Not Started. Rationale: this sequencing isolates cause and effect, making it easier to attribute quality changes and rollback any problematic phase independently. `brainatlas-be/crates/app/src/app.rs:223-257`, `brainatlas-be/crates/infra/src/vectordb.rs:117-137`
- [ ] Task 21. Update service-level and application-level tests to reflect any new retrieval arguments, prompt-construction inputs, and fallback semantics introduced by the earlier phases. Status: Not Started. Rationale: interface drift across app/service/infra layers is a predictable risk whenever retrieval signatures change. `brainatlas-be/crates/app/src/services.rs:122-149`, `brainatlas-be/crates/services/src/services.rs:229-275`, `brainatlas-be/crates/services/src/infra.rs:208-258`
- [ ] Task 22. Add a final regression pass that compares a mixed set of high-evidence and low-evidence regions, verifying that stronger abstention does not inadvertently suppress well-supported content. Status: Not Started. Rationale: quality improvement should increase precision under weak evidence without reducing richness where evidence is strong. `evals-be/crates/app/src/run_eval.rs:25-120`, `evals-be/crates/services/src/state_machine.rs:824-860`

## Verification Criteria

- [ ] Retrieval for a new summary never returns chunks from older summaries for the same region unless an explicitly selected fallback mode is triggered, and that fallback is observable in logs or persisted metadata.
- [ ] Prompt payloads for both RAG and knowledge-only paths include structured region identity beyond `REGION_NAME`, using at least the metadata already present in `RegionMapping`.
- [ ] Sparse-evidence regions produce explicit abstention or uncertainty language instead of fully populated but weakly supported sections.
- [ ] The TTd verification workflow regenerates a fresh TTd summary after the changes and evaluates that exact regenerated summary against the prior TTd baseline, showing either improved groundedness / hallucination-related metrics or a justified shift toward more conservative abstention.
- [ ] Strong-evidence regions retain or improve `claim_groundedness` and do not regress on citation correctness while the contamination fix is active.
- [ ] The default template continues to work for regions lacking reliable type signals, and specialized templates only activate under documented selection rules.
- [ ] Before/after evaluation runs show a measurable reduction in hallucination-like behavior or unsupported specificity without a material loss in supported-content coverage.

## Potential Risks and Mitigations

1. **Retrieval-scope tightening may expose sparse or empty contexts for some runs that were previously “helped” by stale embeddings.**  
   Mitigation: treat this as a quality signal rather than a regression, pair it with stronger abstention behavior, and add explicit fallback modes only when they are intentionally justified and observable.

2. **Region-type-aware templates may misclassify ambiguous structures because the current schema lacks explicit type and ontology fields.**  
   Mitigation: keep template selection heuristic and conservative, require a robust default fallback, and defer broader template branching until a reliable taxonomy source is available.

3. **Prompt tightening may over-correct and produce overly terse summaries even when retrieval is strong.**  
   Mitigation: verify changes first on the scoped TTd regenerate-and-evaluate workflow, then expand to broader cohorts only after confirming that the conservative behavior improves a known failure case without collapsing useful supported content.

4. **Interface changes could ripple across app, service, infra, and test doubles.**  
   Mitigation: land retrieval-contract changes in one cohesive phase, update mocks and service traits together, and gate merges on end-to-end retrieval-scope tests.

5. **Optional reranking could add complexity without meaningful benefit if the dominant issue is actually stale-scope contamination.**  
   Mitigation: postpone reranking until after identity-aware retrieval and prompt revisions have been measured, and only proceed if evaluation deltas still show ranking-specific failure modes.

## Alternative Approaches

1. **Strict current-summary-only retrieval:** Simplest and safest contamination fix, with the strongest isolation guarantees. Trade-off: weaker resilience when a run has few chunks or partial ingestion.
2. **Current-summary-first retrieval with active-summary fallback:** Balances isolation with operational resilience by using `is_active` only as a controlled escape hatch. Trade-off: more complex semantics and more monitoring burden.
3. **Batch-scoped retrieval fallback:** Uses `batch_id` as a middle ground when multiple summaries are produced in one coordinated run. Trade-off: helps multi-step batch workflows but is still broader than strict summary scoping.
4. **Prompt-first improvements without retrieval changes:** Lowest implementation cost, but weaker overall value because it does not remove the root contamination path created by region-only retrieval. Trade-off: easier to ship, less likely to solve the hardest failure mode.
5. **Reranking after identity-aware retrieval:** Useful as a later precision layer if vector-only ordering remains noisy within the correct summary scope. Trade-off: adds latency and complexity, so it should remain optional rather than foundational.

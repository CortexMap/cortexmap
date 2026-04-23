# Detailed Report: Detected Problems in Brain Region Summary Generation

## Scope

This report documents the problems detected while reviewing recent evaluation results and representative summaries in the production database, with a focus on low-scoring outputs and the current summary-generation pipeline.

The investigation covered:
- Latest evaluation scores in `eval_scores`
- Representative failing summaries in `region_summary`
- Retrieval/indexing behavior via `brain_region_embeddings`
- Summary-generation code and prompt templates in the backend

## Executive Summary

The failures are not random. They cluster around a few recurring root causes:

1. **The model is not given enough anatomical context about the target region.**
   The summarization pipeline injects only the region name into prompts, not its parent structure, ontology path, region type, or other disambiguating metadata.

2. **Retrieval is polluted by embeddings from older summaries for the same region.**
   Similar-chunk search filters by `region_id` only, so the model can retrieve chunks from previous ingestion runs and outdated summaries instead of only the current evidence set.

3. **The prompt strongly pressures the model to produce a complete profile even when evidence is weak.**
   The instructions ask for anatomy, functions, disorders, symptoms, and treatments, and to make 4-5 search calls. This encourages over-completion and unsupported synthesis when the evidence is sparse.

4. **Ambiguous or obscure region names are easily misinterpreted.**
   Regions such as Taenia tecta, dorsal part (TTd), Fasciola cinerea (FC), and middle thalamic commissure (mtc) demonstrate name-based drift into incorrect higher-level or unrelated concepts.

5. **Evaluation results are mixing historical inactive summaries with current production summaries.**
   The low-scoring evaluated summaries inspected during this review were inactive rows, not the currently active summaries for those same regions.

These issues combine into a consistent failure mode: the model produces fluent, structured, clinically styled text that is not reliably grounded in the exact region identity or the current evidence set.

## Detailed Problem List

### 1. Missing anatomical and ontology context in prompts

#### Problem
The summarizer is called with the region name and region ID, but the actual prompts only substitute the region name. The model is not explicitly told:
- the parent region
- the parent acronym
- the ontology path
- whether the target is a tract, groove, cortical layer, nucleus, commissure, hippocampal subfield, etc.
- known aliases or disambiguation notes

#### Why this is harmful
For ambiguous or obscure names, the model fills in the gaps from pattern matching or prior knowledge rather than the exact ontology identity of the region.

#### Observed consequences
- **TTd** drifts toward “tectum” / midbrain sensory-reflex descriptions.
- **FC** drifts toward generic “gray matter” language instead of the hippocampal region.
- **mtc** is treated as a richly characterized thalamic commissural structure even when retrieved evidence does not support that level of detail.
- Layer- or parcel-specific regions are likely to drift toward the parent region or a broader canonical structure.

#### Code evidence
- The RAG summarizer receives `region_name` and `region_id`, then replaces only `{{REGION_NAME}}` in the prompts: `brainatlas-be/crates/app/src/app.rs:378-414`
- The RAG system prompt is framed entirely around the region name and does not include parent or ontology metadata: `brainatlas-be/crates/app/prompts/rag_summarize_system.md:1-58`
- The RAG user prompt also uses only the region name: `brainatlas-be/crates/app/prompts/rag_summarize_user.md:1`
- The knowledge-only summarizer similarly injects only the region name into the prompt: `brainatlas-be/crates/app/src/app.rs:303-311`
- The knowledge-only system prompt likewise has no parent/ontology context: `brainatlas-be/crates/app/prompts/knowledge_summarize_system.md:1-53`

### 2. Name-based semantic drift for obscure regions

#### Problem
When the region name is uncommon or resembles a more familiar concept, the model appears to infer meaning from the tokens in the name rather than from exact region identity.

#### Why this is harmful
This causes the summary to describe the wrong structure even when the output is coherent and detailed.

#### Representative cases

##### TTd — Taenia tecta, dorsal part
Observed failure:
- The summary treated TTd as if it belonged to the tectum / midbrain sensory-reflex system.
- It described visual/auditory integration, superior colliculus-like roles, and orienting/reflex functions.

Why this is wrong:
- The model appears to have latched onto the similarity between **tecta** and **tectum**, then generated a profile for a midbrain structure rather than a taenia tecta subdivision.

Eval signals observed in the database:
- `claim_groundedness` was very low.
- `hallucination_rate` was high.
- Relevance and terminology were penalized specifically for misidentifying location and function.

##### FC — Fasciola cinerea
Observed failure:
- The summary described Fasciola cinerea as generic “gray matter” and then produced broad, non-specific neuroscience language.

Why this is wrong:
- The model appears to have interpreted *cinerea* through a generic semantic route and abandoned the exact hippocampal identity of the region.

Eval signals observed in the database:
- Relevance and clinical utility were effectively zero.
- Groundedness was extremely low.
- Hallucination rate was very high.

##### mtc — middle thalamic commissure
Observed failure:
- The summary was highly fluent and detailed, but essentially unsupported by the retrieved evidence.
- It attributed specific sensory, limbic, and motor integration roles despite groundedness collapsing to zero.

Why this matters:
- This is not a simple wording issue. It shows that the pipeline can generate text that sounds expert while being disconnected from the evidence set.

### 3. Retrieval contamination from older embeddings for the same region

#### Problem
Chunk retrieval is filtered by `region_id` only. It does **not** restrict search to the current `summary_id`, current active summary, or current ingestion batch.

#### Why this is harmful
Each region can accumulate embeddings from multiple prior summaries or ingestion cycles. During summarization, the model may retrieve stale, mixed, or previously bad evidence for the same region.

#### Code evidence
- Similarity search only filters on `region_id`: `brainatlas-be/crates/infra/src/vectordb.rs:209-223`
- The service layer stores embeddings with `summary_id`, but retrieval does not use that field: `brainatlas-be/crates/services/src/services.rs:229-257`
- The application inserts the placeholder summary first, then inserts embeddings tied to that summary, which means the system already has the identifier needed for retrieval scoping: `brainatlas-be/crates/app/src/app.rs:213-257`

#### Database evidence observed during investigation
Representative regions showed that older embeddings remain available alongside the current run’s embeddings:
- **TTd**: current evaluated summary had 192 chunks, but 112 additional chunks existed for the same region from other summaries.
- **FC**: current evaluated summary had 95 chunks, but 180 additional chunks existed for the same region from other summaries.
- **mtc**: current evaluated summary had 40 chunks, but 334 additional chunks existed for the same region from other summaries.

#### Consequence
Even if a new run has improved paper coverage or better chunking, retrieval can still surface stale content from prior runs, creating noisy or contradictory evidence for the model.

### 4. Prompt structure encourages over-generation under weak evidence

#### Problem
The RAG prompt strongly instructs the model to produce a full clinical neuroscience profile covering anatomy, function, disorders, symptoms, and treatments, and to make at least 4-5 tool calls.

#### Why this is harmful
For obscure or lightly studied regions, that instruction implicitly rewards the model for filling all sections whether or not retrieved evidence supports them.

#### Code evidence
- The system prompt requires coverage across multiple topic areas and structured sections: `brainatlas-be/crates/app/prompts/rag_summarize_system.md:5-58`
- The user prompt explicitly instructs the model to make 4-5 search calls and create a detailed profile: `brainatlas-be/crates/app/prompts/rag_summarize_user.md:1`

#### Observed consequence
The model tends to produce complete sections on disorders, symptoms, and treatment implications even when the evidence is thin or irrelevant. This is particularly dangerous for small subregions, tracts, grooves, and obscure anatomical structures that may not have rich disorder-specific literature.

### 5. The “insufficient evidence” fallback is too weakly enforced

#### Problem
The prompt includes language telling the model to state when evidence is insufficient rather than guessing, but this instruction is weaker than the broader pressure to produce a complete and polished output.

#### Why this is harmful
The model appears to prefer satisfying the requested structure over abstaining from unsupported claims.

#### Code evidence
- The RAG prompt contains the caution against guessing, but it is embedded inside a larger instruction set that prioritizes complete coverage and polished structure: `brainatlas-be/crates/app/prompts/rag_summarize_system.md:20-58`

#### Observed consequence
The summaries often look complete and clinically useful, but evaluation reveals low groundedness and high hallucination rates.

### 6. Knowledge-only generation has the same identity-disambiguation weakness

#### Problem
When no papers are found, the fallback path asks the model to generate a structured region profile from general knowledge. However, the only injected identifier is the region name.

#### Why this is harmful
For obscure, ambiguous, or highly specific regions, the model is forced to infer the identity of the structure from a name string alone.

#### Code evidence
- The knowledge-only flow constructs messages from a system prompt plus a user request containing only the region name: `brainatlas-be/crates/app/src/app.rs:303-347`
- The knowledge-only prompt encourages the model to provide anatomical and functional inference, but without structured ontology metadata: `brainatlas-be/crates/app/prompts/knowledge_summarize_system.md:1-53`

#### Consequence
The fallback path is vulnerable to the same TTd/FC-style errors even without retrieval contamination, because it lacks firm anatomical grounding at the prompt level.

### 7. Evaluation is surfacing historical inactive summaries, not only current active ones

#### Problem
The low-scoring summary rows examined for TTd, FC, and mtc were inactive summary records, while different summary rows were currently active for those same regions.

#### Why this is important
When reading “latest eval scores,” it is easy to assume the results reflect the current production summary for each region. In practice, the eval table can include historical outputs that are no longer active.

#### Code and data context
- Older summaries are deactivated when a new summary is inserted: `brainatlas-be/crates/infra/src/vectordb.rs:117-136`
- However, evaluation data reviewed during the investigation still included inactive rows for representative failing summaries.

#### Consequence
Quality dashboards or operational reviews may overstate current user-facing problems if they do not distinguish:
- active-summary evals
- historical-summary evals

That said, these inactive rows are still extremely valuable for diagnosis because they reveal systematic failure modes in the generation process.

### 8. The system can produce fluent but ungrounded expert-sounding summaries

#### Problem
The most dangerous failure mode is not crude nonsense, but polished, logically structured text with specific terminology that sounds expert and clinically relevant while lacking evidence support.

#### Why this is harmful
This creates a false sense of quality. Human readers may trust the output because it is coherent and detailed, even when evaluation shows little or no grounding.

#### Evidence
- In the mtc case, raw rubric dimensions such as coherence, terminology, specificity, and relevance scored highly, while groundedness collapsed to zero after gating.
- This indicates that the model can satisfy stylistic and structural expectations without being faithful to the retrieved evidence.

#### Consequence
Without strong grounding controls, polished summaries can bypass manual spot checks and appear “good” unless evaluated systematically.

### 9. Region-type differences are not handled explicitly

#### Problem
The system appears to apply the same general summary template to a wide range of anatomical entity types.

#### Why this is harmful
Different entities need different expectations:
- cortical layers and parcels
- grooves and sulci
- commissures and tracts
- nuclei
- hippocampal subfields
- abstract higher-order groupings

A generic template that always asks for disorders, symptoms, treatments, and clinical implications is ill-suited for many of these categories.

#### Consequence
The model is nudged to overstate the importance and literature coverage of structures that may be primarily anatomical landmarks or narrowly defined subcomponents.

### 10. Acronym handling is weak

#### Problem
Recent evaluated summaries for representative failures showed `acronym_mention = 0`, indicating that acronym usage and region identity anchoring are inconsistent.

#### Why this matters
Acronyms are often crucial for correct region disambiguation in neuroanatomy. If the summary does not consistently anchor on the exact acronym, it becomes easier for the model to drift toward a more familiar nearby concept.

#### Consequence
This likely contributes to failures where the summary is about a plausible-sounding but incorrect concept adjacent to the intended region name.

## Cross-Cutting Root Causes

The observed failures are best explained by the interaction of four underlying causes:

1. **Under-specified region identity**
   - Only the region name is injected into prompts.
   - No parent/ontology/type metadata is provided.

2. **Evidence contamination**
   - Retrieval is region-scoped but not summary-scoped.
   - Old embeddings remain searchable and can mix with current evidence.

3. **Prompt-induced over-completion**
   - The model is strongly encouraged to cover all sections even when evidence is sparse.

4. **Weak abstention behavior**
   - The model is told not to guess, but not constrained strongly enough to refuse unsupported synthesis.

## Highest-Impact Improvement Opportunities

### 1. Scope retrieval to the current summary or batch
The single most important technical fix is to search only embeddings associated with the current summary being generated, or at minimum only the active summary / current batch.

### 2. Pass structured ontology context into prompts
Every summary request should include at least:
- region name
- acronym
- parent region name
- parent acronym
- broader ontology path
- region type/class
- aliases/disambiguation notes if available

### 3. Strengthen the low-evidence behavior
The prompt should explicitly prefer “unknown,” “insufficient evidence,” or “inferred from parent region” over unsupported detail.

### 4. Use type-aware prompting
Commissures, grooves, cortical layers, nuclei, hippocampal subfields, and high-level structures should not all use the same expectations for disorders, symptoms, and interventions.

### 5. Separate historical eval review from active-summary eval review
Operational reporting should make it obvious whether a poor score belongs to:
- the currently active summary
- an older inactive summary retained for history

## Bottom Line

The detected problems are systemic, not isolated. TTd, FC, and mtc are three different manifestations of the same core issue:

- the model is **not anchored tightly enough to exact region identity**, and
- the model is **not constrained tightly enough to the current evidence set**.

As a result, the system can generate summaries that are fluent, confident, and well-structured, but anatomically misidentified, poorly grounded, or contaminated by stale retrieval context.

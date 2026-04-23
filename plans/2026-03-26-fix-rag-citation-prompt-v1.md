# Fix RAG Summarization System Prompt - Citation Overhaul

## Objective

Replace the blanket mandatory citation rule in `brainatlas-be/crates/app/prompts/rag_summarize_system.md` with a tiered citation approach that prevents the LLM from attaching chunk IDs to claims not actually supported by those chunks.

## Problem Analysis

- **Root cause**: Lines 12-18 of the current prompt state "You MUST cite the source chunk for every factual claim." The LLM receives `SimilarChunk` objects containing `chunk_text` (abstracts, metadata summaries — not full papers). When the LLM synthesizes well-known neuroscience knowledge not present in any chunk, it is forced by the prompt to pick the closest-seeming chunk ID, producing misleading citations.
- **Impact**: Readers following `[chunk:UUID]` links find the cited chunk doesn't actually support the claim, eroding trust in the entire citation system.
- **Scope**: Single file change — prompt text only, no Rust code modifications.

## Implementation Plan

- [x] 1. **Replace the "Source citation -- MANDATORY" block (lines 12-18)** with a new "Citation Policy -- Tiered Approach" section containing three tiers:
  - Tier 1: Claims directly stated or strongly implied in a chunk MUST cite with `[chunk:UUID]`
  - Tier 2: Well-established general neuroscience knowledge not found in any chunk may be stated without citation but must not be fabricated
  - Tier 3: If retrieved data is insufficient for a topic, state so explicitly rather than guessing
- [x] 2. **Add a strict anti-hallucination citation rule**: "ONLY cite a chunk if the claim is directly stated or strongly implied in that chunk's text. Never attach a chunk ID to a claim the chunk does not support."
- [x] 3. **Update the example block** to show both a cited claim (Tier 1) and an uncited general-knowledge claim (Tier 2) for clarity.
- [x] 4. **Update the Rules section at the bottom** (lines 54-57) to align with the tiered approach instead of the old blanket mandate.
- [x] 5. **Preserve all other prompt content unchanged**: search strategy, output format sections, scientific terminology rule, and tool-call instructions remain as-is.

## Verification Criteria

- The prompt no longer contains any language mandating citation of "every" factual claim
- Tier 1 (chunk-supported claims) still requires `[chunk:UUID]` citations
- Tier 2 (general knowledge) explicitly permits uncited statements
- Tier 3 (insufficient data) explicitly requires the LLM to disclose gaps
- A strict rule exists: only cite a chunk when the claim is directly stated or strongly implied in that chunk
- The example section demonstrates both cited and uncited claim patterns
- No Rust code changes needed

## Potential Risks and Mitigations

1. **LLM under-cites (stops citing even when chunks support claims)**
   Mitigation: Tier 1 uses strong "MUST cite" language; the example shows proper citation usage first. The prompt makes clear that chunk-supported claims require citation.

2. **LLM abuses Tier 2 to avoid citing anything**
   Mitigation: The prompt explicitly says "prefer citing retrieved evidence over stating general knowledge" and frames uncited statements as an exception, not the default.

3. **Inconsistency with the infra-level prompt at `crates/infra/prompts/summarize_rag_system.md`**
   Mitigation: That prompt (lines 46-49) already uses softer citation language without mandatory `[chunk:UUID]` formatting. The two prompts serve different code paths; no conflict introduced.

## Alternative Approaches

1. **Remove citations entirely**: Simpler but loses all traceability. Not recommended since `get_chunk_source` API exists for resolving citations downstream.
2. **Confidence-scored citations**: Add `[chunk:UUID confidence:high/medium]` tags. More informative but adds complexity the frontend doesn't currently handle.
3. **Post-processing validation**: Add Rust code to strip citations where chunk text doesn't match claim via NLI model. More robust but out of scope (prompt-only change requested).

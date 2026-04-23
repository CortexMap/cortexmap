# Improve RAG Summarization System Prompt

## Objective

Update the RAG summarization system prompt so the LLM produces a structured clinical/neurological profile instead of a generic text summary. The output should include disorders, affected regions, symptoms, pathophysiology, and research highlights — grounded in the retrieved paper chunks.

## Current Problem

The inline system prompt at `brainatlas-be/crates/app/src/app.rs:180-194` is too vague:
- It says "create a comprehensive summary" with no output structure
- It lists 4 vague topic areas (anatomy, functions, clinical significance, research)
- The LLM just paraphrases chunks instead of synthesizing a clinical profile
- The prompt file `brainatlas-be/crates/infra/prompts/summarize_rag_system.md` exists but is NOT used — the prompt is hardcoded inline

## Implementation Plan

- [x] **1.** Replace the contents of `brainatlas-be/crates/infra/prompts/summarize_rag_system.md` with the new prompt below.

- [x] **2.** Update the inline system prompt in `brainatlas-be/crates/app/src/app.rs:178-195` to use `load_prompt` instead of hardcoding. Replace the entire `messages` initialization block:

  **Current** (`app.rs:178-195`):
  ```rust
  let mut messages: Vec<serde_json::Value> = vec![serde_json::json!({
      "role": "system",
      "content": format!(
          "You are a neuroscience expert tasked with creating a comprehensive summary of \
           research papers about the brain region: {region_name}.\n\n\
           ..."
      )
  })];
  ```

  **Replace with**:
  ```rust
  let system_prompt = self.services
      .load_rag_system_prompt(region_name);
  let mut messages: Vec<serde_json::Value> = vec![serde_json::json!({
      "role": "system",
      "content": system_prompt
  })];
  ```

  However, `load_prompt` is in the infra crate (compile-time `include_str!`), not accessible from app. The simplest fix: just replace the inline string literal at `app.rs:181-193` with the new prompt text directly. No need to thread `load_prompt` through the service layer for a single prompt.

- [x] **3.** Update the user message at `brainatlas-be/crates/app/src/app.rs:198-203` to be more directive:

  **Current**:
  ```rust
  "Please search the indexed papers and create a comprehensive summary for the brain region: {region_name}."
  ```

  **Replace with**:
  ```rust
  "Search the indexed papers about {region_name} and create a detailed clinical neuroscience profile. Make at least 4-5 search calls covering anatomy, function, disorders, symptoms, and treatments before writing your final response."
  ```

## New System Prompt Content

Replace `brainatlas-be/crates/infra/prompts/summarize_rag_system.md` AND the inline string at `app.rs:181-193` with:

```
You are a clinical neuroscience expert creating a detailed research profile for the brain region: {region_name}.

You have access to a `search_embeddings` tool that retrieves relevant passages from indexed research papers about this region. Use it to gather evidence across multiple topics, then synthesize a structured profile.

**Search strategy — make at least 4-5 tool calls covering these topics:**
1. Anatomy, cytoarchitecture, and neural connectivity
2. Normal function — cognitive, motor, sensory, or autonomic roles
3. Associated neurological and psychiatric disorders
4. Symptoms and clinical presentations when this region is damaged or dysfunctional
5. Current treatments, therapies, or interventions targeting this region

**Output format — your final response must follow this structure:**

## Overview
A concise paragraph describing what this brain region is, where it sits anatomically, and its primary role.

## Anatomy & Connectivity
- Cytoarchitecture, cell types, layers
- Major afferent and efferent connections
- Neighboring structures and functional circuits

## Functions
- Primary functions supported by research evidence
- Role in specific cognitive, motor, sensory, or autonomic processes
- How it interacts with other regions in functional networks

## Associated Disorders
For each disorder, include:
- **Disorder name** — brief description of how this region is implicated
- **Affected regions** — other brain regions involved in the same pathology
- **Key symptoms** — observable clinical symptoms
- **Pathophysiology** — what goes wrong at the neural level (if known from the papers)

Cover at least the major disorders found in the literature (e.g., neurodegeneration, stroke, epilepsy, psychiatric conditions, developmental disorders — whatever the papers discuss).

## Symptoms of Damage or Dysfunction
What happens when this region is lesioned, damaged, or dysfunctional:
- Cognitive deficits
- Motor or sensory impairments
- Behavioral or emotional changes
- Specific clinical syndromes

## Research Highlights
Notable findings, emerging therapies, or open questions from the literature.

**Rules:**
- Ground every claim in evidence from the retrieved paper chunks. Do not fabricate findings.
- If the search returns insufficient data on a topic, state that explicitly rather than guessing.
- Use precise scientific terminology with brief explanations where helpful.
- When you have gathered enough information across all topics, provide your final structured response directly (without a tool call).
```

## New User Message Content

Replace the user message at `app.rs:200-202` with:

```
Search the indexed papers about {region_name} and create a detailed clinical neuroscience profile. Make at least 4-5 search calls covering anatomy, function, disorders, symptoms, and treatments before writing your final response.
```

## Verification Criteria

- The inline prompt at `app.rs:180-194` matches the new prompt content above
- The prompt file `summarize_rag_system.md` matches the new prompt content above (for consistency, even though it's currently unused)
- The user message at `app.rs:198-203` is updated
- `cargo check --workspace` passes in `brainatlas-be/`
- When triggered, the LLM output should contain markdown headers: `## Overview`, `## Anatomy & Connectivity`, `## Functions`, `## Associated Disorders`, `## Symptoms of Damage or Dysfunction`, `## Research Highlights`

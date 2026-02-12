# System Prompt Architecture Documentation

## Overview

This document outlines the dual-approach system prompt strategy implemented across the CortexMap LLM layer for generating structured brain region information. The system employs two distinct methodologies depending on the use case: RAG-based generation and direct structured output.

---

## Approach 1: RAG-Based System Prompt (`rag_query.py`)

### Purpose
Generate clinically accurate brain region data using **Retrieval-Augmented Generation (RAG)** with local Ollama models and ChromaDB vector store.

### System Prompt Strategy

#### Design Philosophy
The system prompt in `rag_query.py:18-77` follows a **detailed specification approach** that acts as both an instruction manual and quality control mechanism.

#### Key Components

**1. Role Definition**
```
"You are an expert neuroscience knowledge base generator."
```
- Establishes domain expertise
- Sets expectations for medical accuracy

**2. Task Specification**
- Explicit output format: Protocol buffer message for `BrainRegion` entity
- Specific target: Paraventricular hypothalamus (PVN)

**3. Structured Requirements**
The prompt breaks down into numbered sections:

- **ID Field**: Specific identifier format (`pvn_hypothalamus`)
- **Name Field**: Standard anatomical naming convention
- **Location Message**: Detailed anatomical hierarchy
  - Hemisphere specification
  - Lobe/region classification
  - Precise anatomical positioning
  
- **Function & Diseases Message**: Comprehensive clinical information
  - Function description (150-250 words) covering 8 specific aspects
  - Disease description with categorized conditions (endocrine, metabolic, psychiatric, genetic)

**4. Quality Standards**
- Anatomical and clinical accuracy requirements
- Evidence-based medical knowledge mandate
- Literature consistency checks
- Specificity without verbosity
- Clinical relevance criteria
- Logical disease organization

**5. Output Format Directive**
- Clear, structured presentation
- Proper field population
- Nested message hierarchy

### Implementation Details

**Context Integration:**
```python
# rag_query.py:79-89
def format_context(docs_with_scores: List[Tuple]):
    # Builds numbered source references [S1], [S2], etc.
    # Separates chunks with visual delimiters (---)
```

**Message Construction:**
```python
# rag_query.py:160-167
messages = [
    SystemMessage(content=SYSTEM_PROMPT),
    HumanMessage(content=(
        f"Context:\n{context_text}\n\n"
        f"---\n\nQuestion: {args.query_text}\n\n"
        f"Sources (in order): {sources}"
    ))
]
```

**Model Configuration:**
- Model: Configurable (default: `gemma3:latest`)
- Temperature: `0.0` (deterministic output)
- Timeout: `300s` (5 minutes)
- Provider: Ollama (local inference)

### Strengths
✅ **Highly prescriptive** - Minimizes hallucination through detailed specifications  
✅ **Domain-specific** - Tailored for neuroscience/medical context  
✅ **Quality-focused** - Built-in accuracy standards  
✅ **Example-driven** - Provides concrete field values  
✅ **Grounded** - RAG context ensures factual basis

### Limitations
⚠️ **Region-specific** - Hardcoded for PVN (not generalizable)  
⚠️ **Rigid structure** - Difficult to adapt for other brain regions  
⚠️ **Manual maintenance** - Updates require prompt modification

---

## Approach 2: Structured Output with Pydantic AI (`src/query/llm.py`)

### Purpose
Generate structured brain region data using **schema-driven generation** with OpenRouter models and Pydantic validation.

### System Prompt Strategy

#### Design Philosophy
The system prompt in `src/query/llm.py:144-146` and `210-212` follows a **minimal, role-based approach** that relies on schema constraints and structured output capabilities.

#### Key Components

**1. Concise Role Definition**
```python
system_prompt = """You are an expert neuroscience knowledge base. 
Your task is to provide accurate, clinically relevant information about brain regions.
Return structured data with precise anatomical information and comprehensive descriptions."""
```

**2. Schema-Driven Constraints**
Instead of embedding requirements in the prompt, this approach uses **Pydantic models** to enforce structure:

```python
# src/query/llm.py:13-52
class Hemisphere(str, Enum):
    LEFT = "Left"
    RIGHT = "Right"
    BILATERAL = "Bilateral"

class Location(BaseModel):
    hemisphere: Hemisphere
    lobe: str
    anatomical_region: str

class FunctionDiseases(BaseModel):
    function_description: str = Field(
        description="150-250 word paragraph description of functions",
        min_length=1500,
        max_length=2000
    )
    disease_description: str = Field(...)

class BrainRegion(BaseModel):
    name: str
    location: Location
    function_diseases: FunctionDiseases
```

**3. Dynamic User Prompt**
```python
# src/query/llm.py:149-157
user_prompt = f"""Please provide comprehensive information about the {region_name} brain region.

Include:
1. The hemisphere location (Left, Right, or Bilateral)
2. The lobe or major region it belongs to
3. The specific anatomical location
4. A 150-250 word description of its functions
5. A 150-250 word description of associated diseases and dysfunctions
{context}"""
```

**4. Optional Context Loading**
```python
# src/query/llm.py:137-141
if include_context:
    context = load_data_from_folder()
    if context:
        context = f"\n\nReference brain region data:\n{context}"
```

### Implementation Details

**Agent-Based Execution:**
```python
# src/query/llm.py:172-179
agent = Agent(
    model,
    result_type=BrainRegion,  # Schema enforcement
    system_prompt=system_prompt
)

result = agent.run_sync(user_prompt)
return result.data  # Already validated Pydantic model
```

**Model Configuration:**
- Model: Configurable (default: `gpt-oss-120b:free`)
- Provider: OpenRouter (cloud-based inference)
- Output: Native structured output when supported
- Validation: Automatic via Pydantic

### Strengths
✅ **Generalizable** - Works for any brain region  
✅ **Type-safe** - Pydantic validation ensures schema compliance  
✅ **Maintainable** - Schema changes propagate automatically  
✅ **Flexible** - Optional context inclusion  
✅ **Modern** - Leverages structured output APIs  
✅ **Async-ready** - Supports both sync and async execution

### Limitations
⚠️ **Model-dependent** - Requires models with structured output support  
⚠️ **Less prescriptive** - Relies on model's domain knowledge  
⚠️ **API costs** - Uses cloud-based OpenRouter (not fully local)

---

## Design Patterns Identified

### 1. **The Specification Pattern** (RAG Approach)
Embed comprehensive requirements directly in the system prompt when:
- Working with specific, well-defined entities
- Quality standards are critical
- Examples help guide output
- Dealing with specialized domains

### 2. **The Schema-First Pattern** (Structured Output Approach)
Define structure through code and minimal prompts when:
- Building generalizable systems
- Type safety is important
- Schema may evolve
- Multiple similar queries are needed

### 3. **Hybrid Context Strategy**
Both approaches support optional context enrichment:
- RAG: ChromaDB similarity search (semantic retrieval)
- Structured: Markdown file concatenation (reference data)

---

## Recommendations

### When to Use RAG Approach
- **High-stakes medical/clinical applications** where accuracy is paramount
- **Fixed entity generation** where templates are beneficial
- **Offline/local deployment** requirements
- **Auditability** needs (source tracking via retrieval)

### When to Use Structured Output Approach
- **Dynamic applications** requiring varied brain regions
- **API-first architectures** needing type-safe responses
- **Rapid development** with schema evolution
- **Integration** with existing Pydantic-based systems

### Potential Hybrid Approach
Combine the best of both:
1. Use **Pydantic schemas** for validation (maintainability)
2. Use **detailed prompts** for quality (accuracy)
3. Use **RAG retrieval** for grounding (factuality)
4. Use **local models** for privacy + **cloud models** for quality

```python
# Conceptual hybrid
agent = Agent(
    model=OpenRouterModel("high-quality-model"),
    result_type=BrainRegion,  # Schema validation
    system_prompt=DETAILED_NEUROSCIENCE_PROMPT  # Domain expertise
)

# Add RAG context to user prompt
context = retrieve_from_chromadb(region_name)
result = agent.run_sync(f"{context}\n\nGenerate info for {region_name}")
```


# Vector Database Query Integration

## Overview

The `src/query/vectordb.py` module provides RAG (Retrieval-Augmented Generation) functionality for querying brain region information using ChromaDB vector database and formatting responses with the LLM.

## Architecture

```
User Query → ChromaDB Search → Retrieve Context → LLM with Context → Structured BrainRegion
```

**Flow:**
1. **Similarity Search**: Query ChromaDB for relevant brain region documentation
2. **Context Retrieval**: Get top-k most relevant document chunks
3. **LLM Generation**: Pass context to Ollama LLM for structured output
4. **Validation**: Parse and validate response using Pydantic models

## Features

### `VectorDBQueryManager` Class

**Main Methods:**
- `similarity_search()` - Basic vector similarity search
- `similarity_search_with_score()` - Search with relevance scores
- `get_relevant_context()` - Format retrieved documents as context string
- `query_with_context()` - **Main method**: Query with RAG pipeline
- `get_database_stats()` - Get database statistics

### Key Components

1. **Vector Search**: Uses ChromaDB with `nomic-embed-text` embeddings
2. **Context Injection**: Retrieves top-5 relevant document chunks by default
3. **LLM Formatting**: Uses `src/query/llm.py` BrainRegion model for structured output
4. **Pydantic Validation**: Ensures response matches expected schema

## Usage

### 1. Command Line Interface

```bash
# Basic query
python3 src/query/vectordb.py hippocampus

# With verbose output
python3 src/query/vectordb.py hippocampus --verbose

# Custom number of context documents
python3 src/query/vectordb.py "prefrontal cortex" --top-k 10

# Custom database path
python3 src/query/vectordb.py amygdala --chroma-path /path/to/chroma_db
```

### 2. Python API

```python
from src.query.vectordb import query_brain_region_with_vector_db, VectorDBQueryManager

# Simple usage
brain_region = query_brain_region_with_vector_db("hippocampus", verbose=True)
print(f"Name: {brain_region.name}")
print(f"Function: {brain_region.function_diseases.function_description}")

# Advanced usage with manager
manager = VectorDBQueryManager(top_k=10)
result = manager.query_with_context("amygdala", verbose=True)
print(f"Hemisphere: {result.location.hemisphere.value}")
print(f"Lobe: {result.location.lobe}")
```

### 3. Integration with gRPC Service

Update `src/grpc/handlers.py` to use vector database:

```python
from src.query.vectordb import query_brain_region_with_vector_db

def handle_get_or_create_brain_region(region_name: str, config: GRPCConfig):
    # Check cache first
    cached_results = get_brain_region_responses_by_name(region_name)
    if cached_results:
        return cached_results[0], "cache"
    
    # Generate using vector database RAG
    try:
        brain_region = query_brain_region_with_vector_db(
            region_name,
            verbose=True
        )
        
        # Store in database
        record_id = store_brain_region_response(
            query=f"Tell me about the {region_name}",
            brain_region=brain_region,
            model_name=config.llm_model,
            include_context=True
        )
        
        record = get_brain_region_response_by_id(record_id)
        return record, "generated"
    except Exception as e:
        logger.error(f"Vector DB query failed: {e}")
        raise
```

## Environment Variables

```bash
# Required
CHROMA_DB_PATH=/path/to/chroma_db        # ChromaDB storage location
MD_DATA_PATH=/path/to/markdown/docs     # Source documents (for generation)

# Optional
EMBEDDING_MODEL=nomic-embed-text         # Ollama embedding model
```

## Example Output

```bash
$ python3 src/query/vectordb.py hippocampus --verbose

Querying brain region: hippocampus
================================================================================
Searching ChromaDB for: hippocampus
Retrieved 3421 characters of context
Context preview: 
--- Context 1 ---
Source: brain_atlas.md
The hippocampus is a critical structure in the medial temporal lobe...

Querying LLM with context...
✓ Successfully generated response for: Hippocampus

✓ Query successful!

Name: Hippocampus
Hemisphere: Bilateral
Lobe: Temporal Lobe
Anatomical Region: Medial Temporal Lobe

Function Description (1847 chars):
The hippocampus is essential for forming new declarative memories...
[detailed 150-250 word description]

Disease Description (1923 chars):
Damage to the hippocampus results in severe anterograde amnesia...
[detailed 150-250 word description]
```

## Prerequisites

### 1. Create ChromaDB Database

```bash
# Generate vector database from markdown documents
python3 -m src.vectordb.vectordb
```

Expected output:
```
Loading documents from /path/to/data...
Loaded 15 documents
Split 15 documents into 234 chunks.
Creating vector database with 234 chunks...
✓ Saved 234 chunks to /path/to/chroma_db
```

### 2. Ensure Ollama is Running

```bash
# Check Ollama is accessible
ollama list

# Download required model
ollama pull deepseek-r1:8b

# Download embedding model
ollama pull nomic-embed-text
```

## Advantages of Vector DB RAG

### vs. Simple Context Loading

| Approach | Pros | Cons |
|----------|------|------|
| **Load All MD Files** | Simple, always includes everything | Large context, slow, expensive |
| **Vector DB RAG** | Fast, relevant context only, scalable | Requires DB setup, embedding model |

### Benefits

1. **Relevance**: Only retrieves pertinent information
2. **Speed**: Fast similarity search (milliseconds)
3. **Scalability**: Works with large document collections
4. **Accuracy**: Better LLM responses with focused context
5. **Cost**: Reduced token usage (only relevant context sent)

## Error Handling

```python
try:
    result = query_brain_region_with_vector_db("hippocampus")
except FileNotFoundError:
    print("ChromaDB not found. Run: python -m src.vectordb.vectordb")
except Exception as e:
    print(f"Query failed: {e}")
```

## Performance

| Metric | Value |
|--------|-------|
| Vector Search | ~50-100ms |
| Context Retrieval | 5 documents @ ~300 chars each = 1500 chars |
| LLM Generation | ~15-30s (with context) |
| **Total** | ~15-30s |

## Dependencies

Required packages (already in `requirements.txt`):
- `langchain-ollama` - Ollama embeddings
- `langchain-community` - ChromaDB integration
- `chromadb` - Vector database
- `ollama` - LLM API
- `pydantic` - Data validation

## Testing

```bash
# Test vector search
python3 -c "from src.query.vectordb import VectorDBQueryManager; \
  mgr = VectorDBQueryManager(); \
  docs = mgr.similarity_search('hippocampus', k=3); \
  print(f'Found {len(docs)} documents')"

# Test full query
python3 src/query/vectordb.py "prefrontal cortex" --verbose

# Check database stats
python3 -c "from src.query.vectordb import VectorDBQueryManager; \
  mgr = VectorDBQueryManager(); \
  print(mgr.get_database_stats())"
```

## Troubleshooting

### Issue: "ChromaDB not found"
**Solution**: Generate the database first
```bash
python3 -m src.vectordb.vectordb
```

### Issue: "Embedding model not found"
**Solution**: Pull the embedding model
```bash
ollama pull nomic-embed-text
```

### Issue: "No relevant context found"
**Solution**: 
- Check if documents contain information about the brain region
- Try increasing `top_k` parameter
- Verify markdown documents are properly loaded

### Issue: "LLM generation fails"
**Solution**:
- Ensure Ollama is running
- Check deepseek-r1:8b model is downloaded
- Verify sufficient system resources

## Future Enhancements

1. **Hybrid Search**: Combine vector search with keyword search
2. **Reranking**: Add reranking step for better context selection
3. **Caching**: Cache embeddings for faster queries
4. **Metadata Filtering**: Filter by source, date, or other metadata
5. **Async Support**: Add async query methods for better performance

---

**Status**: ✅ Fully Implemented  
**Last Updated**: February 5, 2026

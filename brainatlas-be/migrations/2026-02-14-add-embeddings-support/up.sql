-- 1. Enable pgvector extension
CREATE EXTENSION IF NOT EXISTS vector;

-- 2. Add content_hash to region_summary for deduplication
ALTER TABLE region_summary 
  ADD COLUMN content_hash VARCHAR(64);

-- Index for fast hash lookups
CREATE INDEX idx_region_summary_hash 
  ON region_summary(region_id, content_hash);

COMMENT ON COLUMN region_summary.content_hash IS 
  'SHA-256 hash of all source papers used to generate this summary. Used to avoid reprocessing identical content.';


-- 3. Create embeddings table
CREATE TABLE brain_region_embeddings (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  
  -- Links
  region_id INTEGER NOT NULL,
  summary_id UUID NOT NULL,
  
  -- Chunk data
  chunk_index INTEGER NOT NULL,
  chunk_text TEXT NOT NULL,
  
  -- Vector embedding (1536 dimensions for OpenAI-compatible models)
  embedding vector(1536) NOT NULL,
  
  -- Timestamp
  created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  
  -- Foreign keys
  CONSTRAINT fk_region 
    FOREIGN KEY (region_id) 
    REFERENCES region_mapping(region_id) 
    ON DELETE CASCADE,
    
  CONSTRAINT fk_summary 
    FOREIGN KEY (summary_id) 
    REFERENCES region_summary(id) 
    ON DELETE CASCADE
);

-- Indexes
CREATE INDEX idx_embeddings_region 
  ON brain_region_embeddings(region_id);

CREATE INDEX idx_embeddings_summary 
  ON brain_region_embeddings(summary_id);

-- Vector similarity search index (cosine similarity)
CREATE INDEX idx_embeddings_vector 
  ON brain_region_embeddings 
  USING ivfflat (embedding vector_cosine_ops) 
  WITH (lists = 100);

COMMENT ON TABLE brain_region_embeddings IS 
  'Stores text chunks and their vector embeddings for semantic search. All chunks for a summary are stored together.';

-- 1. Store LLM-generated queries for each region (user-editable)
CREATE TABLE region_queries (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  region_id INTEGER NOT NULL,
  
  -- The search query text
  query_text TEXT NOT NULL,
  
  -- Source of this query
  source TEXT NOT NULL DEFAULT 'llm_generated',
  
  -- Query metadata
  priority INTEGER DEFAULT 0,
  enabled BOOLEAN DEFAULT true,
  
  -- Timestamps
  created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  
  CONSTRAINT source_check CHECK (source IN ('llm_generated', 'user_added', 'user_modified'))
);

-- Index for fast query lookup by region
CREATE INDEX idx_region_queries_region 
  ON region_queries(region_id) 
  WHERE enabled = true;

COMMENT ON TABLE region_queries IS 
  'Stores search queries used to fetch papers for each brain region. Queries are initially LLM-generated but can be user-modified.';


-- 2. Track processing batches (one active batch per region)
CREATE TABLE region_processing_batches (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  region_id INTEGER NOT NULL,
  
  -- Batch status
  status TEXT NOT NULL DEFAULT 'collecting',
  
  -- Track fetch tasks in this batch
  fetch_task_ids BIGINT[] NOT NULL DEFAULT '{}',
  expected_task_count INTEGER NOT NULL,
  
  -- Content deduplication
  content_hash VARCHAR(64),
  
  -- Timestamps
  created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  ready_at TIMESTAMP,
  processing_started_at TIMESTAMP,
  completed_at TIMESTAMP,
  
  -- Result
  summary_id UUID,
  error_message TEXT,
  
  -- Constraints
  CONSTRAINT status_check CHECK (status IN ('collecting', 'ready', 'processing', 'completed', 'failed', 'invalidated')),
  CONSTRAINT expected_count_positive CHECK (expected_task_count > 0)
);

-- Index for finding batches by region and status
CREATE INDEX idx_batches_region_status 
  ON region_processing_batches(region_id, status);

-- Only one active batch per region at a time
-- Note: 'invalidated', 'completed', and 'failed' batches don't count as active
CREATE UNIQUE INDEX idx_one_active_batch_per_region 
  ON region_processing_batches(region_id) 
  WHERE status IN ('collecting', 'ready', 'processing');

COMMENT ON TABLE region_processing_batches IS 
  'Tracks batches of papers being collected and processed for each brain region. One active batch per region at a time.';


-- 3. Add query_generation_limit to config
INSERT INTO orch_config (key, value, description) VALUES
  ('query_generation_limit', '3', 'Number of search queries to generate per brain region');

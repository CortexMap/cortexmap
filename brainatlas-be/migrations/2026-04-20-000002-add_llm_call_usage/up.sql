-- Durable audit record for every outbound LLM call made by brainatlas-be.
-- One row per logical call. See plans/2026-04-20-llm-cost-tracking-v1.md (Phase 4).
--
-- Foreign keys use ON DELETE SET NULL so historical cost data survives even if
-- the originating region_summary or region_processing_batch is deleted (see
-- Risk #6 in the plan).

CREATE TABLE IF NOT EXISTS llm_call_usage (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    endpoint VARCHAR(32) NOT NULL,
    model VARCHAR(256) NOT NULL,
    prompt_tokens INT NOT NULL DEFAULT 0,
    completion_tokens INT NOT NULL DEFAULT 0,
    total_tokens INT NOT NULL DEFAULT 0,
    cost_usd NUMERIC(14, 8) NULL,
    correlation_id VARCHAR(128) NULL,
    region_id INT NULL,
    summary_id UUID NULL,
    batch_id UUID NULL,
    caller_tag VARCHAR(64) NULL,
    request_id VARCHAR(128) NULL
);

-- We intentionally do NOT add referential FKs to region_summary/region_processing_batches
-- because the rows are audit records and should survive the deletion of their
-- originating parents. The plan (Risk #6) proposes ON DELETE SET NULL if FKs
-- are ever added; keeping them as loose pointers for now avoids migration
-- ordering issues and preserves data integrity for accounting.

CREATE INDEX IF NOT EXISTS idx_llm_call_usage_created_at
    ON llm_call_usage (created_at);

CREATE INDEX IF NOT EXISTS idx_llm_call_usage_model_created_at
    ON llm_call_usage (model, created_at);

CREATE INDEX IF NOT EXISTS idx_llm_call_usage_correlation_id
    ON llm_call_usage (correlation_id);

CREATE INDEX IF NOT EXISTS idx_llm_call_usage_region_created_at
    ON llm_call_usage (region_id, created_at);

CREATE INDEX IF NOT EXISTS idx_llm_call_usage_summary
    ON llm_call_usage (summary_id);

CREATE INDEX IF NOT EXISTS idx_llm_call_usage_batch
    ON llm_call_usage (batch_id);

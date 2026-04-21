-- Pricing catalog for LLM models routed through OpenRouter.
-- The `latest_for_model` lookup picks the row with the highest `effective_from`
-- for a given model name. See plans/2026-04-20-llm-cost-tracking-v1.md.

CREATE TABLE IF NOT EXISTS llm_pricing (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model VARCHAR(256) NOT NULL,
    input_price_per_million NUMERIC(12, 6) NOT NULL,
    output_price_per_million NUMERIC(12, 6) NOT NULL,
    embedding_price_per_million NUMERIC(12, 6) NULL,
    currency VARCHAR(8) NOT NULL DEFAULT 'USD',
    effective_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_llm_pricing_model_effective
    ON llm_pricing (model, effective_from);

CREATE INDEX IF NOT EXISTS idx_llm_pricing_model
    ON llm_pricing (model);

-- Seed the current default models. Values reflect OpenRouter published prices
-- as of 2026-04 and MUST be kept in sync via the runbook in
-- plans/2026-04-20-llm-cost-tracking-v1.md (Task 25).
INSERT INTO llm_pricing (model, input_price_per_million, output_price_per_million, embedding_price_per_million)
VALUES
    ('openai/gpt-4o-mini', 0.150000, 0.600000, NULL),
    ('openai/gpt-4o',      2.500000, 10.000000, NULL),
    ('text-embedding-3-small', 0.020000, 0.020000, 0.020000)
ON CONFLICT DO NOTHING;

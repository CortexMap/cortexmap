-- Make summary_id nullable on brain_region_embeddings
-- Embeddings belong to the knowledge base, not necessarily tied to a specific summary.
-- Ingestion-only embeddings (periodic background fetch) won't have a summary yet.

ALTER TABLE brain_region_embeddings ALTER COLUMN summary_id DROP NOT NULL;

-- Drop the existing FK constraint if it exists, then recreate it without CASCADE issues
-- (Diesel joinable! macro just generates query helpers, not actual FK constraints)

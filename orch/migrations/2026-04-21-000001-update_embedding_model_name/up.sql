-- Rename the stored embedding_model config from the bare 'text-embedding-3-small'
-- to the fully-qualified 'openai/text-embedding-3-small' required by Requesty.
-- Only updates rows still holding the old bare name (idempotent).
UPDATE orch_config
SET value = 'openai/text-embedding-3-small'
WHERE key = 'embedding_model'
  AND value = 'text-embedding-3-small';

-- Rollback: revert embedding_model back to the bare name.
UPDATE orch_config
SET value = 'text-embedding-3-small'
WHERE key = 'embedding_model'
  AND value = 'openai/text-embedding-3-small';

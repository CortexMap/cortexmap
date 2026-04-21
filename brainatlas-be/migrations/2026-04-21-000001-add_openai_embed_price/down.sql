-- Rollback: remove the openai/text-embedding-3-small pricing row added by the up migration.
-- The original 'text-embedding-3-small' row from the prior migration is left untouched.
DELETE FROM llm_pricing WHERE model = 'openai/text-embedding-3-small';

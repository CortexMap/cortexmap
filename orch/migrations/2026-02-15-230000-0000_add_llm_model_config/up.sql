-- Add embedding_model and chat_model to orch_config
-- These values configure which LLM models to use for different operations

INSERT INTO orch_config (key, value, description) 
VALUES 
    ('embedding_model', 'text-embedding-3-small', 'LLM model for generating embeddings'),
    ('chat_model', 'openai/gpt-4o-mini', 'LLM model for chat/summarization/query generation')
ON CONFLICT (key) DO UPDATE 
SET description = EXCLUDED.description;

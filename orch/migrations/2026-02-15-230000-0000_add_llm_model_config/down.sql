-- Remove embedding_model and chat_model from orch_config
DELETE FROM orch_config WHERE key IN ('embedding_model', 'chat_model');

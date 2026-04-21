-- Add pricing row for openai/text-embedding-3-small (Requesty/OpenRouter canonical name).
-- The previous migration seeded 'text-embedding-3-small' (without the openai/ prefix).
-- Requesty requires the openai/ prefix; this row covers the new default model name.
INSERT INTO llm_pricing (model, input_price_per_million, output_price_per_million, embedding_price_per_million)
VALUES
    ('openai/text-embedding-3-small', 0.020000, 0.020000, 0.020000)
ON CONFLICT DO NOTHING;

CREATE TABLE papers (
    id BIGSERIAL PRIMARY KEY,
    pmc_id TEXT NOT NULL UNIQUE,
    s3_key TEXT NOT NULL,
    uid TEXT NOT NULL UNIQUE,
    query TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Index for faster lookups by pmc_id
CREATE INDEX idx_papers_pmc_id ON papers(pmc_id);

-- Index for faster lookups by query
CREATE INDEX idx_papers_query ON papers(query);

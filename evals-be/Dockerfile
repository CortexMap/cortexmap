# ── Build evals-be from repo root ─────────────────────────────────────────────
#   docker build -f evals-be/Dockerfile -t evals-be:latest .
#
# Run-time env vars:
#   DATABASE_URL=postgres://...
#   EVAL_VERSION=v0.2.0        (optional; defaults from domain::ConfigKey)
#   EVAL_JUDGE_CHAT_MODEL=...  (optional)
#   EVAL_RUBRIC_CHAT_MODEL=... (optional)
#   EVAL_EMBEDDING_MODEL=...   (optional)
#   EVAL_TOP_K_CHUNKS=5        (optional)
#   EVAL_SIMILARITY_THRESHOLD=0.6 (optional)
#   EVALS_HTTP_ADDR=0.0.0.0:8083
#   CORS_ORIGIN=...
#
# As of 2026-04-19 evals-be is stateless w.r.t. outbound HTTP: LLM calls are
# driven externally by orch. No BRAINATLAS_BASE_URL env var is needed.

FROM rust:bookworm AS builder

RUN apt-get update && apt-get install -y \
    libpq-dev \
    libssl-dev \
    pkg-config \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# evals-be depends on brainatlas-be/{domain,rpc-types} via path = "../brainatlas-be/...".
# Copy both subtrees so the workspace resolves.
COPY evals-be/ /app/evals-be/
COPY brainatlas-be/ /app/brainatlas-be/
COPY proto/ /app/proto/

WORKDIR /app/evals-be
RUN cargo build --release -p evals-server

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    libpq5 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

ARG CORS_ORIGIN=*
ARG EVALS_HTTP_ADDR=0.0.0.0:8083

ENV CORS_ORIGIN=${CORS_ORIGIN}
ENV EVALS_HTTP_ADDR=${EVALS_HTTP_ADDR}

COPY --from=builder /app/evals-be/target/release/evals-server /usr/local/bin/evals-be

EXPOSE 8083

ENTRYPOINT ["/usr/local/bin/evals-be"]

# ── ECR Registry ─────────────────────────────────────────────────────────────
#   Registry : 285560394698.dkr.ecr.us-east-1.amazonaws.com
#   Repository: capstone26t217/brainatlas-be
#   AWS Profile: capstone
#
# ── Usage ───────────────────────────────────────────────────────────────────
#
# 1. Authenticate Docker with ECR (capstone profile):
#   aws ecr get-login-password --profile capstone --region us-east-1 \
#     | docker login --username AWS --password-stdin \
#         285560394698.dkr.ecr.us-east-1.amazonaws.com
#
# 2. Build (no build args needed -- all config is runtime):
#    NOTE: Must be run from repo root to access both brainatlas-be/ and proto/
#   docker build \
#     -f brainatlas-be/Dockerfile \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-be:latest .
#
# 3. Push to ECR:
#   docker push 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-be:latest
#
# 4. Run (inject all config at runtime via --env-file or -e):
#   docker run --env-file .env -p 8081:8081 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-be:latest
#
#   Or individually:
#   docker run \
#     -e DATABASE_URL=postgresql://... \
#     -e CORS_ORIGIN=https://your-frontend.example.com \
#     -e S3_ENDPOINT=http://minio:9000 \
#     -e S3_ACCESS_KEY=... \
#     -e S3_SECRET_KEY=... \
#     -e S3_BUCKET=... \
#     -p 8081:8081 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-be:latest
#
# ────────────────────────────────────────────────────────────────────────────

# ── Stage 1: dependency planner ──────────────────────────────────────────────
FROM rust:bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

# ── Stage 2: generate dependency recipe ──────────────────────────────────────
FROM chef AS planner
COPY brainatlas-be/ .
RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 3: build dependencies (cached layer) ───────────────────────────────
FROM chef AS builder
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    libpq-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build the actual binary
COPY brainatlas-be/ .
COPY proto/ ../proto/
RUN cargo build --release -p server

# ── Stage 4: minimal runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    libpq5 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# No ARG/ENV baking -- all config (DATABASE_URL, CORS_ORIGIN, S3_*,
# BRAINATLAS_HTTP_ADDR, etc.) is supplied at container runtime via
# docker-compose environment:, --env-file, or -e flags.

COPY --from=builder /app/target/release/server /usr/local/bin/brainatlas-be

EXPOSE 8081

ENTRYPOINT ["/usr/local/bin/brainatlas-be"]

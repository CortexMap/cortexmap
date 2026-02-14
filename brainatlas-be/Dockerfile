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
# 2. Build (no secrets needed — DATABASE_URL is supplied at runtime only):
#    NOTE: Must be run from repo root to access both brainatlas-be/ and proto/
#   docker build \
#     --build-arg CORS_ORIGIN \
#     -f brainatlas-be/Dockerfile \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-be:latest .
#
# 3. Push to ECR:
#   docker push 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-be:latest
#
# 4. Run (inject secrets at runtime via --env-file or -e):
#   docker run --env-file .env -p 8081:8081 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-be:latest
#
#   Or individually:
#   docker run \
#     -e DATABASE_URL=postgresql://... \
#     -e CORS_ORIGIN=https://your-frontend.example.com \
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
    && rm -rf /var/lib/apt/lists/*

# All runtime vars (DATABASE_URL, CORS_ORIGIN, HTTP_ADDR) are supplied via
# --env-file or -e at `docker run` time. Only CORS_ORIGIN and HTTP_ADDR
# have build-time defaults as a convenience.
ARG CORS_ORIGIN=*
ARG HTTP_ADDR=0.0.0.0:8081

ENV CORS_ORIGIN=${CORS_ORIGIN}
ENV HTTP_ADDR=${HTTP_ADDR}

COPY --from=builder /app/target/release/server /usr/local/bin/brainatlas-be

EXPOSE 8081

ENTRYPOINT ["/usr/local/bin/brainatlas-be"]

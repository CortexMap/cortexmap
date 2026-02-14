# ── ECR Registry ─────────────────────────────────────────────────────────────
#   Registry : 285560394698.dkr.ecr.us-east-1.amazonaws.com
#   Repository: capstone26t217/fetcher
#   AWS Profile: capstone
#
# ── Usage ───────────────────────────────────────────────────────────────────
#
# 1. Authenticate Docker with ECR (capstone profile):
#   aws ecr get-login-password --profile capstone --region us-east-1 \
#     | docker login --username AWS --password-stdin \
#         285560394698.dkr.ecr.us-east-1.amazonaws.com
#
# 2. Build (non-secret config only — secrets are never baked into the image):
#    NOTE: Must be run from repo root to access both fetcher-be/ and proto/
#   docker build \
#     --build-arg DATABASE_URL \
#     --build-arg S3_ENDPOINT \
#     --build-arg S3_BUCKET \
#     -f fetcher-be/Dockerfile \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest .
#
# 3. Push to ECR:
#   docker push 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest
#
# 4. Run (inject secrets at runtime via --env-file or -e):
#   docker run --env-file .env -p 8080:8080 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest
#
#   Or individually:
#   docker run \
#     -e S3_ACCESS_KEY=... \
#     -e S3_SECRET_KEY=... \
#     -p 8080:8080 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest
#
# ────────────────────────────────────────────────────────────────────────────

# ── Stage 1: dependency planner ──────────────────────────────────────────────
FROM rust:bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

# ── Stage 2: generate dependency recipe ──────────────────────────────────────
FROM chef AS planner
COPY fetcher-be/ .
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
COPY fetcher-be/ .
COPY proto/ ../proto/
RUN cargo build --release

# ── Stage 4: minimal runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    libpq5 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Non-secret config can be baked in at build time via --build-arg.
# Secrets (S3_ACCESS_KEY, S3_SECRET_KEY) must never appear in ARG/ENV —
# supply them exclusively at `docker run` time via --env-file or -e.
ARG DATABASE_URL
ARG S3_ENDPOINT
ARG S3_BUCKET
ARG HTTP_ADDR=0.0.0.0:8080

ENV DATABASE_URL=${DATABASE_URL}
ENV S3_ENDPOINT=${S3_ENDPOINT}
ENV S3_BUCKET=${S3_BUCKET}
ENV HTTP_ADDR=${HTTP_ADDR}

COPY --from=builder /app/target/release/cortexmap-be /usr/local/bin/cortexmap-be

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/cortexmap-be"]

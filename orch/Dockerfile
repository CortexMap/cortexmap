# ── ECR Registry ─────────────────────────────────────────────────────────────
#   Registry : 285560394698.dkr.ecr.us-east-1.amazonaws.com
#   Repository: capstone26t217/orch
#   AWS Profile: capstone
#
# ── Usage ───────────────────────────────────────────────────────────────────
#
# 1. Authenticate Docker with ECR (capstone profile):
#   aws ecr get-login-password --profile capstone --region us-east-1 \
#     | docker login --username AWS --password-stdin \
#         285560394698.dkr.ecr.us-east-1.amazonaws.com
#
# 2. Build (no secrets needed — DATABASE_URL and service addresses are supplied
#    at runtime only):
#    NOTE: Must be run from repo root to access both orch/ and proto/
#   docker build \
#     --build-arg CORS_ORIGIN \
#     -f orch/Dockerfile \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/orch:latest .
#
# 3. Push to ECR:
#   docker push 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/orch:latest
#
# 4. Run (inject secrets at runtime via --env-file or -e):
#   docker run --env-file .env -p 8081:8081 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/orch:latest
#
#   Or individually:
#   docker run \
#     -e DATABASE_URL=postgresql://... \
#     -e FETCHER_HTTP_ADDR=http://fetcher-be:8080 \
#     -e BRAINATLAS_HTTP_ADDR=http://brainatlas-be:8081 \
#     -e CORS_ORIGIN=https://your-frontend.example.com \
#     -p 8081:8081 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/orch:latest
#
# ────────────────────────────────────────────────────────────────────────────

# ── Stage 1: dependency planner ──────────────────────────────────────────────
FROM rust:bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

# ── Stage 2: generate dependency recipe ──────────────────────────────────────
FROM chef AS planner
COPY orch/ .
RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 3: build dependencies (cached layer) ───────────────────────────────
FROM chef AS builder
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    libpq-dev \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build the actual binary
COPY orch/ .
COPY proto/ ../proto/
RUN cargo build --release -p server

# ── Stage 4: minimal runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    libpq5 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# All runtime vars (DATABASE_URL, FETCHER_HTTP_ADDR, BRAINATLAS_HTTP_ADDR) are
# supplied via --env-file or -e at `docker run` time. Only CORS_ORIGIN and
# ORCH_HTTP_ADDR have build-time defaults as a convenience.
ARG CORS_ORIGIN=*
ARG ORCH_HTTP_ADDR=0.0.0.0:8081

ENV CORS_ORIGIN=${CORS_ORIGIN}
ENV ORCH_HTTP_ADDR=${ORCH_HTTP_ADDR}

COPY --from=builder /app/target/release/server /usr/local/bin/orch

# Run as an unprivileged user instead of the default root.
# The service binds to an unprivileged port (>1024), so no extra capabilities
# are required.
RUN groupadd --system --gid 10001 app \
 && useradd  --system --uid 10001 --gid app --no-create-home --shell /usr/sbin/nologin app
USER app:app

EXPOSE 8081

ENTRYPOINT ["/usr/local/bin/orch"]

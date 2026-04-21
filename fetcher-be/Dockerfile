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
# 2. Build (no config/secrets baked in — everything is injected at runtime):
#    NOTE: Must be run from repo root to access both fetcher-be/ and proto/
#   docker build \
#     -f fetcher-be/Dockerfile \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest .
#
# 3. Push to ECR:
#   docker push 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest
#
# 4. Run (inject all config at runtime via --env-file or -e):
#   docker run --env-file .env -p 8080:8080 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest
#
#   Or individually:
#   docker run \
#     -e DATABASE_URL=postgres://... \
#     -e S3_ENDPOINT=http://... \
#     -e S3_ACCESS_KEY=... \
#     -e S3_SECRET_KEY=... \
#     -e S3_BUCKET=cortexmap \
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
    curl \
    && rm -rf /var/lib/apt/lists/*

# All configuration is injected at container runtime via environment variables.
# Required: DATABASE_URL, S3_ENDPOINT, S3_ACCESS_KEY, S3_SECRET_KEY, S3_BUCKET
# Optional: FETCHER_HTTP_ADDR (defaults to 0.0.0.0:8080)

COPY --from=builder /app/target/release/cortexmap-be /usr/local/bin/cortexmap-be

# Run as an unprivileged user instead of the default root.
# The service binds to an unprivileged port (>1024), so no extra capabilities
# are required.
RUN groupadd --system --gid 10001 app \
 && useradd  --system --uid 10001 --gid app --no-create-home --shell /usr/sbin/nologin app
USER app:app

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/cortexmap-be"]

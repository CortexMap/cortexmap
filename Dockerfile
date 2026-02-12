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
# 2a. Build (read args from host environment):
#   docker build \
#     --build-arg DATABASE_URL \
#     --build-arg S3_ENDPOINT \
#     --build-arg S3_ACCESS_KEY \
#     --build-arg S3_SECRET_KEY \
#     --build-arg S3_BUCKET \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest .
#
# 2b. Build (read args from a .env file):
#   export $(grep -v '^#' .env | xargs) && docker build \
#     --build-arg DATABASE_URL \
#     --build-arg S3_ENDPOINT \
#     --build-arg S3_ACCESS_KEY \
#     --build-arg S3_SECRET_KEY \
#     --build-arg S3_BUCKET \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest .
#
# 3. Push to ECR:
#   docker push 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest
#
# Run (supply secrets at container start instead of baking them into the image):
#   docker run --env-file .env -p 8080:8080 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/fetcher:latest
#
# ────────────────────────────────────────────────────────────────────────────

# ── Stage 1: dependency planner ──────────────────────────────────────────────
FROM rust:bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

# ── Stage 2: generate dependency recipe ──────────────────────────────────────
FROM chef AS planner
COPY . .
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
COPY . .
RUN cargo build --release -p cortexmap-be

# ── Stage 4: minimal runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    libpq5 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Declare build args — when passed via --build-arg, they are baked into the
# image as environment variables. Omit --build-arg at build time and supply
# them at `docker run` with --env-file instead if you prefer not to bake
# secrets into the image layers.
ARG DATABASE_URL
ARG S3_ENDPOINT
ARG S3_ACCESS_KEY
ARG S3_SECRET_KEY
ARG S3_BUCKET
ARG HTTP_ADDR=0.0.0.0:8080

ENV DATABASE_URL=${DATABASE_URL}
ENV S3_ENDPOINT=${S3_ENDPOINT}
ENV S3_ACCESS_KEY=${S3_ACCESS_KEY}
ENV S3_SECRET_KEY=${S3_SECRET_KEY}
ENV S3_BUCKET=${S3_BUCKET}
ENV HTTP_ADDR=${HTTP_ADDR}

COPY --from=builder /app/target/release/cortexmap-be /usr/local/bin/cortexmap-be

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/cortexmap-be"]

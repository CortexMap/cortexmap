# ── ECR Registry ─────────────────────────────────────────────────────────────
#   Registry : 285560394698.dkr.ecr.us-east-1.amazonaws.com
#   Repository: capstone26t217/brainatlas-fe
#   AWS Profile: capstone
#
# ── Usage ───────────────────────────────────────────────────────────────────
#
# 1. Authenticate Docker with ECR (capstone profile):
#   aws ecr get-login-password --profile capstone --region us-east-1 \
#     | docker login --username AWS --password-stdin \
#         285560394698.dkr.ecr.us-east-1.amazonaws.com
#
# 2. Build:
#    NOTE: Must be run from repo root so the build context is correct.
#   docker build \
#     --build-arg VITE_API_BASE_URL=https://capstone.ssdd.dev/orch/api \
#     -f brainatlas-fe/Dockerfile \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-fe:latest .
#
# 3. Push to ECR:
#   docker push 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-fe:latest
#
# 4. Run:
#   docker run -p 3000:80 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/brainatlas-fe:latest
#
# ────────────────────────────────────────────────────────────────────────────

# ── Stage 1: build ───────────────────────────────────────────────────────────
FROM node:22-alpine AS builder

WORKDIR /app

# Vite env vars are baked in at build time
ARG VITE_API_BASE_URL
ARG VITE_DEBUG=false

ENV VITE_API_BASE_URL=${VITE_API_BASE_URL}
ENV VITE_DEBUG=${VITE_DEBUG}

# Install dependencies (cached layer)
COPY brainatlas-fe/package.json brainatlas-fe/package-lock.json ./
RUN npm ci

# Build the production bundle
COPY brainatlas-fe/ .
RUN npm run build

# ── Stage 2: serve with nginx ────────────────────────────────────────────────
FROM nginx:alpine AS runtime

# Remove default nginx config
RUN rm /etc/nginx/conf.d/default.conf

# Add custom nginx config for SPA routing
COPY --from=builder /app/nginx.conf /etc/nginx/conf.d/default.conf

# Copy built assets
COPY --from=builder /app/dist /usr/share/nginx/html

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]

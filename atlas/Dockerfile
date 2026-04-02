# ── ECR Registry ─────────────────────────────────────────────────────────────
#   Registry : 285560394698.dkr.ecr.us-east-1.amazonaws.com
#   Repository: capstone26t217/atlas
#   AWS Profile: capstone
#
# ── Usage ───────────────────────────────────────────────────────────────────
#
# 1. Authenticate Docker with ECR (capstone profile):
#   aws ecr get-login-password --profile capstone --region us-east-1 \
#     | docker login --username AWS --password-stdin \
#         285560394698.dkr.ecr.us-east-1.amazonaws.com
#
# 2. Build (pass API base URL as build arg if needed):
#   docker build \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/atlas:latest \
#     atlas/
#
#   With custom API URL:
#   docker build \
#     --build-arg VITE_API_BASE_URL=https://capstone.ssdd.dev/orch/api \
#     -t 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/atlas:latest \
#     atlas/
#
# 3. Push to ECR:
#   docker push 285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/atlas:latest
#
# 4. Run:
#   docker run -p 8082:80 \
#     285560394698.dkr.ecr.us-east-1.amazonaws.com/capstone26t217/atlas:latest
#
# ────────────────────────────────────────────────────────────────────────────

# ── Stage 1: install dependencies (cached layer) ────────────────────────────
FROM node:22-alpine AS deps
WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci

# ── Stage 2: build static assets ────────────────────────────────────────────
FROM deps AS builder
WORKDIR /app

# Accept API base URL at build time (baked into the JS bundle by Vite)
ARG VITE_API_BASE_URL=https://capstone.ssdd.dev/orch/api
ENV VITE_API_BASE_URL=${VITE_API_BASE_URL}

COPY . .
RUN npm run build

# ── Stage 3: minimal runtime image ──────────────────────────────────────────
FROM nginx:alpine AS runtime

# Remove default nginx site
RUN rm -rf /usr/share/nginx/html/*

# Copy built assets
COPY --from=builder /app/dist /usr/share/nginx/html

# SPA fallback: serve index.html for all routes
RUN printf 'server {\n\
    listen 80;\n\
    server_name _;\n\
    root /usr/share/nginx/html;\n\
    index index.html;\n\
\n\
    location / {\n\
        try_files $uri $uri/ /index.html;\n\
    }\n\
\n\
    # Cache static assets aggressively\n\
    location /assets/ {\n\
        expires 1y;\n\
        add_header Cache-Control "public, immutable";\n\
    }\n\
}\n' > /etc/nginx/conf.d/default.conf

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]

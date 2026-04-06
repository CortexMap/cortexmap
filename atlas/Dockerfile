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
# 2. Build:
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

# ── Stage 2: fetch 3D meshes + build static assets ──────────────────────────
FROM deps AS builder
WORKDIR /app

# VITE_API_BASE_URL is baked into the JS bundle at build time by Vite.
# Override with --build-arg if deploying to a different backend.
ARG VITE_API_BASE_URL=https://capstone.ssdd.dev/orch/api
ENV VITE_API_BASE_URL=${VITE_API_BASE_URL}

COPY . .

# Download Allen CCF 2017 structure meshes (OBJ files) for 3D viewer.
# ~840 meshes, ~228 MB -- served as static assets to bypass CORS.
RUN npm run fetch-meshes

# TypeScript compile + Vite production build
RUN npm run build

# ── Stage 3: minimal runtime image ──────────────────────────────────────────
FROM nginx:alpine AS runtime

RUN rm -rf /usr/share/nginx/html/*

# Copy Vite build output + pre-fetched 3D meshes
COPY --from=builder /app/dist /usr/share/nginx/html
COPY --from=builder /app/public/meshes /usr/share/nginx/html/meshes

# nginx config: SPA fallback, asset caching, Allen mesh CORS proxy.
# The /allen-meshes/ proxy is a fallback for any OBJ mesh not pre-fetched
# at build time (download.alleninstitute.org blocks browser CORS).
RUN printf 'server {\n\
    listen 80;\n\
    server_name _;\n\
    root /usr/share/nginx/html;\n\
    index index.html;\n\
\n\
    # SPA: serve index.html for all client-side routes (/, /3d)\n\
    location / {\n\
        try_files $uri $uri/ /index.html;\n\
    }\n\
\n\
    # Vite hashed assets -- cache forever\n\
    location /assets/ {\n\
        expires 1y;\n\
        add_header Cache-Control "public, immutable";\n\
    }\n\
\n\
    # Pre-fetched 3D meshes -- cache 24h\n\
    location /meshes/ {\n\
        expires 1d;\n\
        add_header Cache-Control "public";\n\
    }\n\
\n\
    # Proxy Allen mesh downloads (bypasses CORS)\n\
    location /allen-meshes/ {\n\
        proxy_pass https://download.alleninstitute.org/informatics-archive/current-release/mouse_ccf/annotation/ccf_2017/structure_meshes/;\n\
        proxy_ssl_server_name on;\n\
        proxy_set_header Host download.alleninstitute.org;\n\
        proxy_set_header Accept-Encoding "";\n\
        proxy_hide_header Access-Control-Allow-Origin;\n\
        add_header Access-Control-Allow-Origin "*" always;\n\
        add_header Cache-Control "public, max-age=86400";\n\
    }\n\
}\n' > /etc/nginx/conf.d/default.conf

EXPOSE 80

ENTRYPOINT ["nginx", "-g", "daemon off;"]

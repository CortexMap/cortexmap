# CortexMap BFF - Backend-for-Frontend

Connects the React frontend (`app-fe`) to the Python gRPC BrainRegionService (RAG backend) via HTTP REST.

## Architecture

```
Frontend (React, port 3001)
    ↓ HTTP GET /api/brain-regions
CortexMap BFF (Rust, port 8080)
    ↓ gRPC
Python BrainRegionService (port 5005)
```

## Prerequisites

1. **Python gRPC server** (from `llm_layer` on `rag` branch):
   - Ensure `llm_layer` with `comm.proto` and `server.py` is available
   - Start: `python llm_layer/src/grpc/server.py` (listens on port 5005)

2. **Rust toolchain** for building the BFF

## Running

### 1. Start the Python gRPC server (RAG backend)

From the project root, on a branch that has `llm_layer/src/grpc/`:

```bash
cd llm_layer
python src/grpc/server.py
# Listens on 0.0.0.0:5005
```

### 2. Start the BFF (Rust)

```bash
cargo run -p cortexmap-bff
# Listens on 0.0.0.0:8080
```

Optional env vars:

- `BRAIN_REGION_GRPC_ADDR` - gRPC server address (default: `http://127.0.0.1:5005`)
- `BFF_HTTP_ADDR` - HTTP listen address (default: `0.0.0.0:8080`)

### 3. Start the frontend

```bash
cd app-fe
npm install
npm start
# Vite dev server on port 3001, proxies /api to BFF
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/health | Health check |
| GET | /api/brain-regions | All brain regions |
| GET | /api/brain-regions?q=query | Search brain regions |

## Production

Set `VITE_API_URL` to your BFF URL when building the frontend:

```bash
VITE_API_URL=https://your-bff-host.com npm run build
```

Ensure CORS is configured (BFF allows all origins by default for development).

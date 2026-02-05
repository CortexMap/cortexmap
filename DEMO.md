# CortexMap Demo – How to Run and See It Working

This guide walks you through running the full stack and viewing the demo.

---

## Prerequisites

1. **Rust** – for the BFF
2. **Node.js** – for the frontend
3. **Python 3.10+** – for the gRPC server
4. **PostgreSQL** – the Python server reads from a `brain_region_responses` table

---

## Architecture

```
Browser (localhost:3001)
    ↓ REST
BFF (localhost:8080)
    ↓ gRPC
Python gRPC server (localhost:5005)
    ↓ SQL
PostgreSQL (brain_region_responses table)
```

---

## Step 1: Set Up PostgreSQL

The Python server expects PostgreSQL with database `llmlayer` and table `brain_region_responses`.

Create `llm_layer/.env`:

```
DB_HOST=localhost
DB_PORT=5433
DB_NAME=llmlayer
DB_USER=llmlayer
DB_PASSWORD=llmlayer_dev
```

Start PostgreSQL (e.g. via Docker):

```bash
# Example with Docker (adjust port if needed)
docker run -d --name cortexmap-pg \
  -e POSTGRES_DB=llmlayer \
  -e POSTGRES_USER=llmlayer \
  -e POSTGRES_PASSWORD=llmlayer_dev \
  -p 5433:5432 \
  postgres:15
```

Initialize the schema:

```bash
cd llm_layer
python src/db/schema.py
```

Seed sample data (optional; run once):

```bash
cd llm_layer
python scripts/seed_brain_regions.py
```

---

## Step 2: Start the Python gRPC Server (Terminal 1)

```bash
cd llm_layer
pip install -r requirements.txt
python -m src.grpc.server
```

Expected output: server listening on port 5005.

---

## Step 3: Start the BFF (Terminal 2)

```bash
cargo run -p cortexmap-bff
```

Expected output: BFF listening on http://0.0.0.0:8080.

---

## Step 4: Start the Frontend (Terminal 3)

```bash
cd app-fe
npm install
npm start
```

Vite should open http://localhost:3001 in the browser.

---

## Step 5: See the Demo

1. The app loads and calls `/api/brain-regions`.
2. Vite proxies this to the BFF; the BFF calls the Python gRPC server.
3. If the database has data, brain region cards appear.
4. Use the search bar; after typing, search requests go to `/api/brain-regions?q=...`.

---

## Quick Checks

### 1. BFF health

```bash
curl http://localhost:8080/api/health
# Expected: {"status":"ok"}
```

### 2. All brain regions (via BFF)

```bash
curl http://localhost:8080/api/brain-regions
# Expected: JSON array of brain regions (or [] if DB is empty)
```

### 3. Search

```bash
curl "http://localhost:8080/api/brain-regions?q=amygdala"
```

---

## If You See "No data to show"

The table `brain_region_responses` is empty. You can:

1. **Seed sample data** – run `llm_layer/scripts/seed_brain_regions.py` (see Step 1).
2. **Populate via LLM** – run queries using `query_brain_region_with_vector_db` or `brain_region_query` and store results with `store_brain_region_response`.

---

## Troubleshooting

| Issue | What to check |
|-------|----------------|
| BFF fails to start | Python gRPC server running on port 5005 |
| Frontend shows error | BFF running on port 8080, Vite proxy pointing to 8080 |
| Python server fails | PostgreSQL running, `.env` set, `brain_region_responses` table exists |
| Empty results | Table has rows; try seeding script or running LLM queries |

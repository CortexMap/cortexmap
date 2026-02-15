#!/bin/bash

# Load environment variables
export $(grep -v '^#' /home/ssdd/RustroverProjects/cortexmap/.env | xargs)

# Kill existing processes
echo "Stopping existing services..."
pkill -f "target/debug/cortexmap-be" || true
pkill -f "target/debug/server" || true
pkill -f "target/debug/brainatlas-be" || true
sleep 2

# Start fetcher-be on port 8080
echo "Starting fetcher-be on port 8080..."
cd /home/ssdd/RustroverProjects/cortexmap/fetcher-be
HTTP_ADDR="0.0.0.0:8080" \
RUST_LOG=info \
cargo run --bin cortexmap-be > /tmp/fetcher.log 2>&1 &
FETCHER_PID=$!

# Start brainatlas-be on port 8081
echo "Starting brainatlas-be on port 8081..."
cd /home/ssdd/RustroverProjects/cortexmap/brainatlas-be
HTTP_ADDR="0.0.0.0:8081" \
RUST_LOG=info \
cargo run --bin server > /tmp/brainatlas.log 2>&1 &
BRAINATLAS_PID=$!

# Start orch on port 8082
echo "Starting orch on port 8082..."
cd /home/ssdd/RustroverProjects/cortexmap/orch
HTTP_ADDR="0.0.0.0:8082" \
FETCHER_URL="http://localhost:8080" \
BRAINATLAS_URL="http://localhost:8081" \
RUST_LOG=info,orch=debug \
cargo run --bin server > /tmp/orch.log 2>&1 &
ORCH_PID=$!

echo "Waiting for services to start..."
sleep 5

echo ""
echo "=== Service Status ==="
echo "Fetcher   (8080): PID $FETCHER_PID"
echo "BrainAtlas (8081): PID $BRAINATLAS_PID"
echo "Orch       (8082): PID $ORCH_PID"
echo ""

# Test health endpoints
echo "Testing health endpoints..."
echo -n "Fetcher:    "
curl -s http://localhost:8080/fetcher-be/health | jq -r '.status // "ERROR"' || echo "FAILED"
echo -n "BrainAtlas: "
curl -s http://localhost:8081/brainatlas-be/health | jq -r '.status // "ERROR"' || echo "FAILED"
echo -n "Orch:       "
curl -s http://localhost:8082/orch/health | jq -r '.status // "ERROR"' || echo "FAILED"

echo ""
echo "Logs available at:"
echo "  /tmp/fetcher.log"
echo "  /tmp/brainatlas.log"
echo "  /tmp/orch.log"
echo ""
echo "To monitor logs: tail -f /tmp/orch.log"
echo "To stop services: pkill -f 'target/debug/(cortexmap-be|server|brainatlas-be)'"

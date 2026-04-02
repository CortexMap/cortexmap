#!/usr/bin/env bash
# Setup test infrastructure and run integration tests

set -e

echo "🚀 Starting test infrastructure..."
docker-compose -f docker-compose.test.yml up -d

echo "⏳ Waiting for services to be healthy..."
timeout 60 bash -c 'until docker-compose -f docker-compose.test.yml ps | grep -q "healthy"; do sleep 2; done'

echo "✅ Test infrastructure is ready!"

redis-cli -h localhost -p 6380 FLUSHDB || true

# Set test environment variables
export TEST_MODE=true
export DATABASE_URL="postgresql://test_user:test_password@localhost:5433/test_db"
export REDIS_URL="redis://localhost:6380"
export S3_ENDPOINT="http://localhost:9000"
export S3_ACCESS_KEY="test_access_key"
export S3_SECRET_KEY="test_secret_key"
export S3_BUCKET="test-bucket"

echo "🧪 Running migrations..."
cd fetcher-be && diesel migration run --database-url "$DATABASE_URL" && cd ..
cd brainatlas-be && diesel migration run --database-url "$DATABASE_URL" && cd ..
cd orch && diesel migration run --database-url "$DATABASE_URL" && cd ..

echo "🧪 Running integration tests..."
cargo test --workspace --test '*' -- --test-threads=1 --nocapture

echo "🧹 Cleaning up..."
docker-compose -f docker-compose.test.yml down -v

echo "✅ All tests passed!"

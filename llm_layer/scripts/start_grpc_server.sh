#!/bin/bash
# Startup script for gRPC server

set -e

echo "🚀 Starting Brain Region gRPC Server..."
echo ""

# Load environment variables if .env exists
if [ -f .env ]; then
    echo "Loading environment variables from .env..."
    export $(grep -v '^#' .env | xargs)
fi

# Check if database is accessible
echo "Checking database connection..."
python -c "from src.db.schema import get_connection; get_connection().close(); print('✓ Database connection OK')" || {
    echo "❌ Database connection failed!"
    echo "Make sure PostgreSQL is running and environment variables are set."
    exit 1
}

# Initialize database schema if needed
echo "Initializing database schema..."
python src/db/schema.py

# Generate protobuf code
echo "Generating protobuf code..."
./scripts/generate_proto.sh

# Start gRPC server
echo ""
echo "Starting gRPC server..."
python -m src.grpc.server

#!/bin/bash
# Wrapper script to run the gRPC example client

cd "$(dirname "$0")/.."
python3 src/grpc/example_client.py "$@"

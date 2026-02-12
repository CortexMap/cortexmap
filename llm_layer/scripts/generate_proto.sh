#!/bin/bash
# Script to generate Python code from protobuf definitions

set -e

echo "Generating protobuf code..."

# Navigate to project root (assuming script is in llm_layer/scripts/)
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"
cd "$PROJECT_ROOT"

python -m grpc_tools.protoc \
  -I./proto/llm \
  -I./proto \
  --python_out=./llm_layer/src/grpc \
  --grpc_python_out=./llm_layer/src/grpc \
  --pyi_out=./llm_layer/src/grpc \
  ./proto/llm/brain.proto

echo "✓ Protobuf code generated successfully in llm_layer/src/grpc/"

# Fix import paths in generated files
echo "Fixing import paths..."
sed -i 's/^import brain_pb2/from . import brain_pb2/' llm_layer/src/grpc/brain_pb2_grpc.py 2>/dev/null || true

echo "✓ Import paths fixed"
echo ""
echo "Generated files:"
ls -lh llm_layer/src/grpc/brain_pb2* 2>/dev/null || echo "Note: Files will be brain_pb2.py and brain_pb2_grpc.py"

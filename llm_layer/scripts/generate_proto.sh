#!/bin/bash
# Script to generate Python code from protobuf definitions

set -e

echo "Generating protobuf code..."

python -m grpc_tools.protoc \
  -I./src \
  --python_out=./src/grpc/generated \
  --grpc_python_out=./src/grpc/generated \
  --pyi_out=./src/grpc/generated \
  ./src/brain_region.proto

echo "✓ Protobuf code generated successfully in src/grpc/generated/"

# Fix import paths in generated files
echo "Fixing import paths..."
sed -i 's/^import brain_region_pb2/from . import brain_region_pb2/' src/grpc/generated/brain_region_pb2_grpc.py

echo "✓ Import paths fixed"
echo ""
echo "Generated files:"
ls -lh src/grpc/generated/brain_region_pb2*

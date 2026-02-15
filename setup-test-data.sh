#!/usr/bin/env bash
# Helper script to upload test data to MinIO for integration tests

set -e

MINIO_ENDPOINT="http://localhost:9000"
MINIO_ACCESS_KEY="test_access_key"
MINIO_SECRET_KEY="test_secret_key"
BUCKET_NAME="test-bucket"

echo "📦 Setting up test data in MinIO..."

# Configure mc (MinIO client)
mc alias set testminio "$MINIO_ENDPOINT" "$MINIO_ACCESS_KEY" "$MINIO_SECRET_KEY"

# Create bucket if it doesn't exist
mc mb testminio/$BUCKET_NAME --ignore-existing

# Create sample test files
mkdir -p /tmp/test-papers

cat > /tmp/test-papers/paper1.txt << 'EOF'
Title: Neural Mechanisms of Memory Consolidation

Abstract:
This study investigates the neural mechanisms underlying memory consolidation
in the hippocampus. Our findings demonstrate that synaptic plasticity plays
a crucial role in the transfer of information from short-term to long-term memory.

Introduction:
Memory consolidation is a fundamental process in neuroscience...

Methods:
We used electrophysiological recordings to measure neural activity...

Results:
Our results show significant changes in synaptic strength during sleep cycles...

Conclusion:
These findings provide new insights into memory formation and consolidation.
EOF

cat > /tmp/test-papers/paper2.md << 'EOF'
# Synaptic Plasticity in Learning

## Abstract
This research explores the relationship between synaptic plasticity and learning processes.

## Introduction
Synaptic plasticity is the ability of synapses to strengthen or weaken over time...

## Key Findings
- Long-term potentiation (LTP) is essential for memory formation
- Synaptic depression plays a role in forgetting
- Neuroplasticity continues throughout the lifespan

## Conclusion
Understanding synaptic plasticity mechanisms opens new avenues for treating
memory disorders and enhancing cognitive function.
EOF

cat > /tmp/test-papers/abstract.txt << 'EOF'
Neuroplasticity and Cognitive Enhancement: A Review

This comprehensive review examines the current understanding of neuroplasticity
and its implications for cognitive enhancement. We discuss various techniques
including cognitive training, physical exercise, and pharmacological interventions
that have been shown to enhance brain plasticity and improve cognitive performance.
EOF

# Upload test files to MinIO
echo "📤 Uploading test files..."
mc cp /tmp/test-papers/paper1.txt testminio/$BUCKET_NAME/papers/TEST_PMC001/paper.txt
mc cp /tmp/test-papers/paper2.md testminio/$BUCKET_NAME/papers/TEST_PMC002/paper.md
mc cp /tmp/test-papers/abstract.txt testminio/$BUCKET_NAME/papers/TEST_PMC003/abstract.txt

# Verify uploads
echo "✅ Test data uploaded successfully!"
mc ls testminio/$BUCKET_NAME/papers/ --recursive

echo ""
echo "Test S3 keys available:"
echo "  - papers/TEST_PMC001/paper.txt"
echo "  - papers/TEST_PMC002/paper.md"
echo "  - papers/TEST_PMC003/abstract.txt"

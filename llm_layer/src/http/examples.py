"""
Frontend Integration Examples for Brain Atlas HTTP API

This file provides examples for integrating with the HTTP API from various
frontend technologies using binary protobuf.
"""

# ==============================================================================
# Python Example (using requests library)
# ==============================================================================

PYTHON_EXAMPLE = """
import requests
import comm_pb2

# Create request
request = comm_pb2.SearchBrainRegionRequest()
request.query = "hippocampus"

# Serialize to binary
request_data = request.SerializeToString()

# Send HTTP POST
response = requests.post(
    "http://localhost:5006/search-brain-region",
    data=request_data,
    headers={"Content-Type": "application/x-protobuf"}
)

# Deserialize response
pb_response = comm_pb2.SearchBrainRegionResponse()
pb_response.ParseFromString(response.content)

# Access data
for entry in pb_response.entries:
    print(f"Region: {entry.region_name}")
    print(f"Hemisphere: {entry.hemisphere}")
"""

# ==============================================================================
# JavaScript/TypeScript Example (using protobuf.js)
# ==============================================================================

JAVASCRIPT_EXAMPLE = """
// Install: npm install protobufjs

import protobuf from 'protobufjs';

// Load proto definitions
const root = await protobuf.load('comm.proto');
const SearchBrainRegionRequest = root.lookupType('comm.SearchBrainRegionRequest');
const SearchBrainRegionResponse = root.lookupType('comm.SearchBrainRegionResponse');

// Create and encode request
const request = SearchBrainRegionRequest.create({ query: 'hippocampus' });
const requestBuffer = SearchBrainRegionRequest.encode(request).finish();

// Send HTTP POST
const response = await fetch('http://localhost:5006/search-brain-region', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/x-protobuf',
  },
  body: requestBuffer,
});

// Decode response
const responseBuffer = await response.arrayBuffer();
const pbResponse = SearchBrainRegionResponse.decode(new Uint8Array(responseBuffer));

// Access data
console.log('Status:', pbResponse.status);
pbResponse.entries.forEach(entry => {
  console.log('Region:', entry.regionName);
  console.log('Hemisphere:', entry.hemisphere);
});
"""

# ==============================================================================
# TypeScript with Axios Example
# ==============================================================================

TYPESCRIPT_AXIOS_EXAMPLE = """
// Install: npm install axios protobufjs
// Install types: npm install @types/protobufjs

import axios from 'axios';
import protobuf from 'protobufjs';

interface BrainRegionEntry {
  id: number;
  query: string;
  queryTimestamp: number;
  regionName: string;
  hemisphere: string;
  lobe: string;
  anatomicalRegion: string;
  functionDescription: string;
  diseaseDescription: string;
  createdAt: number;
  updatedAt: number;
}

interface SearchResponse {
  entries: BrainRegionEntry[];
  status: string;
  errorMessage: string;
}

async function searchBrainRegion(query: string): Promise<SearchResponse> {
  // Load proto
  const root = await protobuf.load('comm.proto');
  const RequestType = root.lookupType('comm.SearchBrainRegionRequest');
  const ResponseType = root.lookupType('comm.SearchBrainRegionResponse');
  
  // Create request
  const request = RequestType.create({ query });
  const requestBuffer = RequestType.encode(request).finish();
  
  // Send request
  const response = await axios.post(
    'http://localhost:5006/search-brain-region',
    requestBuffer,
    {
      headers: { 'Content-Type': 'application/x-protobuf' },
      responseType: 'arraybuffer',
    }
  );
  
  // Decode response
  const pbResponse = ResponseType.decode(new Uint8Array(response.data));
  return ResponseType.toObject(pbResponse) as unknown as SearchResponse;
}

// Usage
const results = await searchBrainRegion('hippocampus');
console.log('Found entries:', results.entries.length);
"""

# ==============================================================================
# React Hook Example
# ==============================================================================

REACT_HOOK_EXAMPLE = """
// Install: npm install protobufjs

import { useState, useEffect } from 'react';
import protobuf from 'protobufjs';

// Load proto once
let protoRoot: protobuf.Root | null = null;

async function loadProto() {
  if (!protoRoot) {
    protoRoot = await protobuf.load('/path/to/comm.proto');
  }
  return protoRoot;
}

export function useBrainRegionSearch(query: string) {
  const [data, setData] = useState(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(null);
  
  useEffect(() => {
    if (!query) return;
    
    async function fetchData() {
      setLoading(true);
      setError(null);
      
      try {
        const root = await loadProto();
        const RequestType = root.lookupType('comm.SearchBrainRegionRequest');
        const ResponseType = root.lookupType('comm.SearchBrainRegionResponse');
        
        // Create request
        const request = RequestType.create({ query });
        const requestBuffer = RequestType.encode(request).finish();
        
        // Fetch
        const response = await fetch('http://localhost:5006/search-brain-region', {
          method: 'POST',
          headers: { 'Content-Type': 'application/x-protobuf' },
          body: requestBuffer,
        });
        
        // Decode
        const arrayBuffer = await response.arrayBuffer();
        const pbResponse = ResponseType.decode(new Uint8Array(arrayBuffer));
        const data = ResponseType.toObject(pbResponse);
        
        setData(data);
      } catch (err) {
        setError(err);
      } finally {
        setLoading(false);
      }
    }
    
    fetchData();
  }, [query]);
  
  return { data, loading, error };
}

// Usage in component
function BrainRegionSearch() {
  const [query, setQuery] = useState('hippocampus');
  const { data, loading, error } = useBrainRegionSearch(query);
  
  if (loading) return <div>Loading...</div>;
  if (error) return <div>Error: {error.message}</div>;
  
  return (
    <div>
      <input value={query} onChange={(e) => setQuery(e.target.value)} />
      {data?.entries?.map(entry => (
        <div key={entry.id}>
          <h3>{entry.regionName}</h3>
          <p>{entry.hemisphere} - {entry.lobe}</p>
        </div>
      ))}
    </div>
  );
}
"""

# ==============================================================================
# cURL Examples
# ==============================================================================

CURL_EXAMPLES = """
# Health Check
curl http://localhost:5006/health

# Root endpoint (API info)
curl http://localhost:5006/

# Search Brain Region (binary protobuf)
# First, create a binary protobuf file:
python3 << EOF
import sys
sys.path.append('src')
import comm_pb2

request = comm_pb2.SearchBrainRegionRequest()
request.query = "hippocampus"

with open('/tmp/search_request.pb', 'wb') as f:
    f.write(request.SerializeToString())
EOF

# Then send it:
curl -X POST http://localhost:5006/search-brain-region \\
  -H "Content-Type: application/x-protobuf" \\
  --data-binary @/tmp/search_request.pb \\
  --output /tmp/search_response.pb

# Decode the response:
python3 << EOF
import sys
sys.path.append('src')
import comm_pb2

with open('/tmp/search_response.pb', 'rb') as f:
    response = comm_pb2.SearchBrainRegionResponse()
    response.ParseFromString(f.read())
    print(f"Status: {response.status}")
    print(f"Entries: {len(response.entries)}")
    for entry in response.entries:
        print(f"  - {entry.region_name} ({entry.hemisphere})")
EOF

# Get All Brain Regions
python3 << EOF
import sys
sys.path.append('src')
import comm_pb2

request = comm_pb2.GetAllBrainRegionsRequest()
with open('/tmp/getall_request.pb', 'wb') as f:
    f.write(request.SerializeToString())
EOF

curl -X POST http://localhost:5006/get-all-brain-regions \\
  -H "Content-Type: application/x-protobuf" \\
  --data-binary @/tmp/getall_request.pb \\
  --output /tmp/getall_response.pb
"""

# ==============================================================================
# Testing Content-Type Validation
# ==============================================================================

CONTENT_TYPE_TEST = """
# Test with wrong content type (should return 415)
curl -X POST http://localhost:5006/search-brain-region \\
  -H "Content-Type: application/json" \\
  -d '{"query": "test"}' \\
  -v

# Expected response:
# HTTP/1.1 415 Unsupported Media Type
# {
#   "error": "Unsupported Media Type",
#   "detail": "Content-Type must be one of: application/x-protobuf, application/octet-stream",
#   "received": "application/json"
# }

# Test with correct content type
curl -X POST http://localhost:5006/search-brain-region \\
  -H "Content-Type: application/x-protobuf" \\
  --data-binary @/tmp/search_request.pb \\
  -v
"""

# ==============================================================================
# Proto File Generation for Frontend
# ==============================================================================

PROTO_GENERATION = """
# Generate JavaScript protobuf files for frontend

# Option 1: Using protobufjs-cli
npm install -g protobufjs-cli
pbjs -t static-module -w commonjs -o comm.js src/grpc/comm.proto
pbts -o comm.d.ts comm.js

# Option 2: Using protoc with js plugin
protoc --js_out=import_style=commonjs,binary:. src/grpc/comm.proto

# Option 3: Runtime loading (no generation needed)
# Just copy comm.proto to your frontend public directory
# and load it at runtime with protobuf.load()
"""

# ==============================================================================
# Environment Variables
# ==============================================================================

ENV_VARS = """
# Server Configuration
HTTP_HOST=0.0.0.0                    # Host to bind to
HTTP_PORT=5006                       # Port to listen on
CORS_ORIGINS=http://localhost:3000,http://localhost:8080  # Allowed origins

# Example .env file
cat > .env << 'EOF'
# HTTP Server
HTTP_HOST=0.0.0.0
HTTP_PORT=5006
CORS_ORIGINS=*

# Database (already configured)
DB_HOST=localhost
DB_PORT=5433
DB_NAME=brain_atlas
DB_USER=postgres
DB_PASSWORD=postgres
EOF
"""

# ==============================================================================
# Docker Deployment
# ==============================================================================

DOCKER_EXAMPLE = """
# Add to docker-compose.yml

services:
  # ... existing services ...
  
  http-api:
    build: .
    container_name: brain-atlas-http
    ports:
      - "5006:5006"
    environment:
      - HTTP_HOST=0.0.0.0
      - HTTP_PORT=5006
      - CORS_ORIGINS=*
      - DB_HOST=postgres
      - DB_PORT=5432
      - DB_NAME=brain_atlas
      - DB_USER=postgres
      - DB_PASSWORD=postgres
    command: python src/http/server.py
    depends_on:
      - postgres
    networks:
      - brain-atlas-network

# Or run standalone:
docker run -p 5006:5006 \\
  -e HTTP_PORT=5006 \\
  -e CORS_ORIGINS=http://localhost:3000 \\
  brain-atlas-http
"""

# ==============================================================================
# CORS Configuration
# ==============================================================================

CORS_CONFIG = """
# Development (allow all origins)
CORS_ORIGINS=*

# Production (specific origins)
CORS_ORIGINS=https://brain-atlas.example.com,https://app.example.com

# Multiple origins with ports
CORS_ORIGINS=http://localhost:3000,http://localhost:8080,https://production.com

# The server automatically:
# - Allows all HTTP methods
# - Allows all headers
# - Allows credentials
# - Exposes Content-Type header
"""


if __name__ == '__main__':
    print("Brain Atlas HTTP API - Frontend Integration Guide")
    print("="*60)
    print("\nAvailable examples:")
    print("  - PYTHON_EXAMPLE")
    print("  - JAVASCRIPT_EXAMPLE")
    print("  - TYPESCRIPT_AXIOS_EXAMPLE")
    print("  - REACT_HOOK_EXAMPLE")
    print("  - CURL_EXAMPLES")
    print("  - CONTENT_TYPE_TEST")
    print("  - PROTO_GENERATION")
    print("  - ENV_VARS")
    print("  - DOCKER_EXAMPLE")
    print("  - CORS_CONFIG")
    print("\nImport this module and access examples via their variable names.")

# Brain Atlas HTTP API with Binary Protobuf Support

HTTP API layer for the Brain Atlas service that accepts and sends binary protobuf data, enabling web browsers and HTTP clients to interact with the service using efficient binary serialization.

## Overview

This HTTP API mirrors the functionality of the gRPC service (`src/grpc/server.py`) while using standard HTTP/1.1 transport with binary protobuf request/response bodies. This enables:

- Web browser access via fetch/XMLHttpRequest
- No special gRPC client libraries required
- CORS support for cross-origin requests
- Standard HTTP status codes and headers
- Efficient binary protobuf serialization

## Architecture

```
Frontend → HTTP POST (binary protobuf) → FastAPI Server → db.repository → PostgreSQL
                                              ↓
                                        comm_pb2 (shared protobuf definitions)
                                              ↑
Frontend ← HTTP Response (binary protobuf) ← Serialized Response
```

## Endpoints

### `POST /search-brain-region`

Search for brain regions by query term.

- **Request**: Binary protobuf `SearchBrainRegionRequest`
- **Response**: Binary protobuf `SearchBrainRegionResponse`
- **Content-Type**: `application/x-protobuf` or `application/octet-stream`

### `POST /get-all-brain-regions`

Retrieve all brain region entries from the database.

- **Request**: Binary protobuf `GetAllBrainRegionsRequest` (empty message)
- **Response**: Binary protobuf `GetAllBrainRegionsResponse`
- **Content-Type**: `application/x-protobuf` or `application/octet-stream`

### `GET /health`

Health check endpoint.

- **Response**: JSON `{"status": "healthy", "service": "brain-atlas-http"}`

### `GET /`

API information and documentation links.

### `GET /docs`

Interactive API documentation (Swagger UI).

## Installation

### Dependencies

Install required packages:

```bash
pip install fastapi uvicorn[standard] python-multipart
```

Or install from requirements.txt:

```bash
pip install -r requirements.txt
```

### Verify Installation

```bash
python src/http/server.py
```

Expected output:
```
============================================================
Brain Atlas HTTP API with Binary Protobuf Support
============================================================
Host: 0.0.0.0
Port: 5006
CORS Origins: ['*']
Documentation: http://0.0.0.0:5006/docs
============================================================
INFO:     Started server process
INFO:     Waiting for application startup.
INFO:     Application startup complete.
INFO:     Uvicorn running on http://0.0.0.0:5006
```

## Usage

### Python Client

```python
import requests
import sys
sys.path.append('src')
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
print(f"Status: {pb_response.status}")
for entry in pb_response.entries:
    print(f"Region: {entry.region_name} ({entry.hemisphere})")
```

### JavaScript/TypeScript Client

```javascript
import protobuf from 'protobufjs';

// Load proto definitions
const root = await protobuf.load('comm.proto');
const SearchRequest = root.lookupType('comm.SearchBrainRegionRequest');
const SearchResponse = root.lookupType('comm.SearchBrainRegionResponse');

// Create and encode request
const request = SearchRequest.create({ query: 'hippocampus' });
const requestBuffer = SearchRequest.encode(request).finish();

// Send HTTP POST
const response = await fetch('http://localhost:5006/search-brain-region', {
  method: 'POST',
  headers: { 'Content-Type': 'application/x-protobuf' },
  body: requestBuffer,
});

// Decode response
const responseBuffer = await response.arrayBuffer();
const pbResponse = SearchResponse.decode(new Uint8Array(responseBuffer));

// Access data
console.log('Status:', pbResponse.status);
pbResponse.entries.forEach(entry => {
  console.log(`Region: ${entry.regionName} (${entry.hemisphere})`);
});
```

### cURL Example

```bash
# Create binary protobuf request
python3 << EOF
import sys
sys.path.append('src')
import comm_pb2

request = comm_pb2.SearchBrainRegionRequest()
request.query = "hippocampus"

with open('/tmp/request.pb', 'wb') as f:
    f.write(request.SerializeToString())
EOF

# Send request
curl -X POST http://localhost:5006/search-brain-region \
  -H "Content-Type: application/x-protobuf" \
  --data-binary @/tmp/request.pb \
  --output /tmp/response.pb

# Decode response
python3 << EOF
import sys
sys.path.append('src')
import comm_pb2

with open('/tmp/response.pb', 'rb') as f:
    response = comm_pb2.SearchBrainRegionResponse()
    response.ParseFromString(f.read())
    print(f"Status: {response.status}")
    for entry in response.entries:
        print(f"  - {entry.region_name}")
EOF
```

## Testing

### Run Test Client

The included test client demonstrates all API endpoints:

```bash
python src/http/test_client.py
```

With custom parameters:

```bash
python src/http/test_client.py --host localhost --port 5006 --query "cortex"
```

### Manual Testing

```bash
# Terminal 1: Start server
python src/http/server.py

# Terminal 2: Run tests
python src/http/test_client.py
```

## Configuration

### Environment Variables

Configure via `.env` file or environment variables:

- **HTTP_HOST**: Host to bind to (default: `0.0.0.0`)
- **HTTP_PORT**: Port to listen on (default: `5006`)
- **CORS_ORIGINS**: Comma-separated allowed origins (default: `*`)

Example `.env`:

```bash
HTTP_HOST=0.0.0.0
HTTP_PORT=5006
CORS_ORIGINS=http://localhost:3000,http://localhost:8080
```

### CORS Configuration

The server supports CORS out of the box. For production, specify allowed origins:

```bash
# Development (allow all)
CORS_ORIGINS=*

# Production (specific origins)
CORS_ORIGINS=https://brain-atlas.example.com,https://app.example.com
```

## Protobuf Message Definitions

Messages are defined in `src/grpc/comm.proto`:

### BrainRegionEntry

```protobuf
message BrainRegionEntry {
    int32 id = 1;
    string query = 2;
    int64 query_timestamp = 3;      // Unix milliseconds
    string region_name = 4;
    string hemisphere = 5;           // "Left", "Right", "Bilateral"
    string lobe = 6;
    string anatomical_region = 7;
    string function_description = 8;
    string disease_description = 9;
    int64 created_at = 10;          // Unix milliseconds
    int64 updated_at = 11;          // Unix milliseconds
}
```

### Request/Response Messages

- `SearchBrainRegionRequest { string query }`
- `SearchBrainRegionResponse { repeated BrainRegionEntry entries, string status, string error_message }`
- `GetAllBrainRegionsRequest {}` (empty)
- `GetAllBrainRegionsResponse { repeated BrainRegionEntry entries, int32 total_count, string status, string error_message }`

## Error Handling

### HTTP Status Codes

- **200 OK**: Request successful (check protobuf status field for operation status)
- **400 Bad Request**: Malformed protobuf data
- **415 Unsupported Media Type**: Invalid Content-Type header
- **500 Internal Server Error**: Server error (check error_message in protobuf response)

### Protobuf Status Field

All response messages include a `status` field:

- `"success"`: Operation completed successfully
- `"not_found"`: No results found (search only)
- `"error"`: An error occurred (see error_message)

### Error Response Example

Even on HTTP 500, the response is a valid protobuf message:

```python
# HTTP 500 response still contains protobuf
pb_response = comm_pb2.SearchBrainRegionResponse()
pb_response.ParseFromString(response.content)

if pb_response.status == "error":
    print(f"Error: {pb_response.error_message}")
```

## Performance

### Binary Protobuf vs JSON

Binary protobuf provides significant advantages:

- **Size**: 50-70% smaller payloads than equivalent JSON
- **Speed**: Faster serialization/deserialization
- **Type Safety**: Strong typing with schema validation
- **Compatibility**: Same format used by gRPC service

### Benchmarks

Typical response sizes for BrainRegionEntry:

- **Protobuf**: ~300-400 bytes per entry
- **JSON**: ~600-800 bytes per entry
- **Savings**: ~50% reduction in network transfer

## Comparison with gRPC Service

| Feature | HTTP API | gRPC API |
|---------|----------|----------|
| Transport | HTTP/1.1 | HTTP/2 |
| Port | 5006 | 5005 |
| Format | Binary Protobuf | Binary Protobuf |
| CORS | ✅ Built-in | ❌ Requires proxy |
| Browser | ✅ Native fetch | ⚠️ Needs gRPC-Web |
| Streaming | ❌ Not supported | ✅ Supported |
| Documentation | ✅ Swagger UI | ⚠️ Requires tools |

## Deployment

### Standalone

```bash
python src/http/server.py
```

### With Uvicorn

```bash
uvicorn src.http.server:app --host 0.0.0.0 --port 5006
```

### Docker

```dockerfile
FROM python:3.11-slim

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY src/ ./src/
EXPOSE 5006

CMD ["python", "src/http/server.py"]
```

### Docker Compose

```yaml
services:
  http-api:
    build: .
    ports:
      - "5006:5006"
    environment:
      - HTTP_PORT=5006
      - CORS_ORIGINS=*
      - DB_HOST=postgres
      - DB_PORT=5432
    depends_on:
      - postgres
```

## Troubleshooting

### Content-Type Errors (415)

**Problem**: `Unsupported Media Type`

**Solution**: Ensure Content-Type header is set:
```javascript
headers: { 'Content-Type': 'application/x-protobuf' }
```

### Protobuf Decode Errors

**Problem**: `Failed to parse request`

**Solution**: Verify protobuf serialization:
```python
# Correct
request_data = request.SerializeToString()

# Incorrect
request_data = str(request)  # Don't use str()
```

### CORS Errors in Browser

**Problem**: `No 'Access-Control-Allow-Origin' header`

**Solution**: Configure CORS_ORIGINS:
```bash
CORS_ORIGINS=http://localhost:3000
```

### Empty Response

**Problem**: No entries returned

**Solution**: Check database has data and query matches:
```bash
# Check database
psql -h localhost -p 5433 -U postgres brain_atlas
SELECT COUNT(*) FROM brain_region_responses;
```

## Development

### Code Structure

```
src/http/
├── __init__.py          # Module exports
├── server.py            # FastAPI application and endpoints
├── test_client.py       # Testing client
└── examples.py          # Frontend integration examples
```

### Adding New Endpoints

1. Define protobuf message in `src/grpc/comm.proto`
2. Regenerate Python bindings: `python -m grpc_tools.protoc ...`
3. Add endpoint in `server.py`:

```python
@app.post("/new-endpoint")
async def new_endpoint(request: Request):
    body = await request.body()
    pb_request = deserialize_request(body, comm_pb2.NewRequest)
    
    # Process request
    result = process_data(pb_request)
    
    # Create response
    pb_response = comm_pb2.NewResponse(data=result)
    response_data = serialize_response(pb_response)
    
    return Response(content=response_data, media_type="application/x-protobuf")
```

## Frontend Integration

See `examples.py` for complete integration examples:

- Python with requests
- JavaScript with fetch
- TypeScript with Axios
- React hooks
- cURL commands

## License

Same as parent project.

## Support

For issues or questions:
1. Check logs: Server logs all requests and errors
2. Verify protobuf definitions match between client and server
3. Test with included test_client.py
4. Review examples.py for integration patterns

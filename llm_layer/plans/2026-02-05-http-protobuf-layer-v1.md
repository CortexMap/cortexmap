# HTTP Layer with Protobuf Binary Data Support

## Objective

Create a new HTTP server layer in `src/http/server.py` that accepts and sends binary protobuf data to the frontend. The HTTP layer will mirror the existing gRPC service functionality (`src/grpc/server.py:13-84`) while using HTTP transport with protobuf binary serialization, enabling web browsers and HTTP clients to interact with the Brain Atlas service using the same protobuf message definitions (`src/grpc/comm.proto:1-52`).

## Analysis Summary

### Current Architecture
The existing gRPC server (`src/grpc/server.py`) provides:
- **SearchBrainRegion**: Query-based search using `comm_pb2.SearchBrainRegionRequest` → `comm_pb2.SearchBrainRegionResponse`
- **GetAllBrainRegions**: Retrieve all entries using `comm_pb2.GetAllBrainRegionsRequest` → `comm_pb2.GetAllBrainRegionsResponse`
- **Data Source**: PostgreSQL via `db.repository.search_by_query()` and `db.repository.get_all_responses()`
- **Port**: 5005 (gRPC)

### Protobuf Message Structures
From `src/grpc/comm.proto:15-43`:
- `BrainRegionEntry`: 11 fields including id, region data, timestamps
- `SearchBrainRegionRequest`: Single query field
- `SearchBrainRegionResponse`: entries array, status, error_message
- `GetAllBrainRegionsResponse`: entries array, total_count, status, error_message

### Key Technical Details
- **Timestamp Format**: Unix epoch milliseconds (int64) at `server.py:24,31`
- **Database Fields**: id, query, query_timestamp, region_name, hemisphere, lobe, anatomical_region, function_description, disease_description, created_at, updated_at
- **Error Handling**: Status-based responses ("success", "not_found", "error")
- **Protobuf Version**: 6.33.5 (from `requirements.txt:18`)

## Implementation Plan

### Phase 1: HTTP Server Foundation

- [ ] **1.1 Create HTTP Module Directory Structure**
  - Create `src/http/` directory to mirror `src/grpc/` organization
  - Create `src/http/__init__.py` for module exports
  - Rationale: Maintains architectural consistency with existing `src/grpc/` pattern and provides clear separation of transport layers

- [ ] **1.2 Implement FastAPI Server with Protobuf Support**
  - Create `src/http/server.py` with FastAPI application instance
  - Add CORS middleware configuration for frontend access (allow origins, methods, headers)
  - Configure binary request/response content types (`application/x-protobuf`, `application/octet-stream`)
  - Set up server to listen on port 5006 (avoiding conflict with gRPC port 5005)
  - Rationale: FastAPI provides async support, automatic API documentation, and native binary data handling. Port 5006 provides logical adjacency to gRPC port while avoiding conflicts with existing services (PostgreSQL:5433, Adminer:8080, pgAdmin:5050)

- [ ] **1.3 Add Protobuf Imports and Path Configuration**
  - Import protobuf bindings: `comm_pb2` from `src/grpc/comm_pb2.py`
  - Add parent directory to sys.path (mirror pattern from `server.py:4-6`)
  - Import repository functions: `search_by_query`, `get_all_responses` from `db.repository`
  - Rationale: Reuses existing protobuf definitions and database layer, ensuring consistency with gRPC implementation

### Phase 2: Binary Protobuf HTTP Endpoints

- [ ] **2.1 Implement POST /search-brain-region Endpoint**
  - Accept binary protobuf request body (Content-Type: application/x-protobuf)
  - Deserialize request using `comm_pb2.SearchBrainRegionRequest.FromString()`
  - Extract query parameter from deserialized request object
  - Call `search_by_query(request.query)` to retrieve database results
  - Transform database rows to `comm_pb2.BrainRegionEntry` objects (mirror logic from `server.py:20-35`)
  - Convert timestamp fields using `int(row['timestamp'].timestamp() * 1000)` pattern
  - Create `comm_pb2.SearchBrainRegionResponse` with entries, status, and error_message
  - Serialize response using `.SerializeToString()` method
  - Return binary response with Content-Type: application/x-protobuf
  - Rationale: Mirrors gRPC `SearchBrainRegion` functionality while using HTTP POST for client compatibility

- [ ] **2.2 Implement POST /get-all-brain-regions Endpoint**
  - Accept binary protobuf request body (Content-Type: application/x-protobuf)
  - Deserialize request using `comm_pb2.GetAllBrainRegionsRequest.FromString()`
  - Call `get_all_responses()` to retrieve all database records
  - Transform database rows to `comm_pb2.BrainRegionEntry` objects (mirror logic from `server.py:55-70`)
  - Calculate total_count from entries list length
  - Create `comm_pb2.GetAllBrainRegionsResponse` with entries, total_count, status, error_message
  - Serialize response using `.SerializeToString()` method
  - Return binary response with Content-Type: application/x-protobuf
  - Rationale: Provides equivalent functionality to gRPC `GetAllBrainRegions` method for HTTP clients

- [ ] **2.3 Add Error Handling and Exception Management**
  - Wrap endpoint logic in try-except blocks (mirror pattern from `server.py:43-48`)
  - Catch protobuf parsing errors (DecodeError, ParseError)
  - Catch database exceptions from repository layer
  - Return appropriate error responses with status="error" and populated error_message
  - Log errors using Python logging module
  - Return HTTP 400 for client errors (malformed protobuf), HTTP 500 for server errors
  - Rationale: Ensures robust error handling consistent with gRPC implementation and provides clear feedback for debugging

### Phase 3: Helper Functions and Utilities

- [ ] **3.1 Create Protobuf Serialization Helper Functions**
  - Implement `deserialize_request(body: bytes, message_class)` function for request parsing
  - Implement `serialize_response(response_object)` function for response encoding
  - Add type hints for better IDE support and type checking
  - Rationale: Reduces code duplication, centralizes serialization logic, and improves maintainability

- [ ] **3.2 Implement Database-to-Protobuf Transformation Utility**
  - Create `row_to_brain_region_entry(row: Dict) -> comm_pb2.BrainRegionEntry` function
  - Handle timestamp conversions (datetime → milliseconds int64)
  - Mirror exact transformation logic from `server.py:21-33`
  - Add null/None value handling for optional fields
  - Rationale: Ensures consistent data transformation between gRPC and HTTP layers, reducing maintenance burden

- [ ] **3.3 Add Request Validation Middleware**
  - Validate Content-Type header is application/x-protobuf or application/octet-stream
  - Return HTTP 415 Unsupported Media Type for incorrect Content-Type
  - Add request size limits to prevent DoS attacks (e.g., max 10MB)
  - Rationale: Improves security and provides clear error messages for API consumers

### Phase 4: Server Lifecycle and Configuration

- [ ] **4.1 Implement Server Startup and Configuration**
  - Create `serve()` function (mirror pattern from `server.py:87-95`)
  - Add Uvicorn server configuration with host='0.0.0.0', port=5006
  - Configure worker count for concurrent request handling
  - Add startup event handler to log server start message
  - Implement graceful shutdown handling
  - Rationale: Ensures production-ready server configuration with proper lifecycle management

- [ ] **4.2 Add Environment Variable Configuration**
  - Support HTTP_PORT environment variable (default: 5006)
  - Support HTTP_HOST environment variable (default: 0.0.0.0)
  - Support CORS_ORIGINS environment variable for frontend URLs
  - Load configuration using python-dotenv (already in `requirements.txt:21`)
  - Rationale: Enables flexible deployment configuration without code changes

- [ ] **4.3 Create Main Entry Point**
  - Add `if __name__ == '__main__': serve()` block
  - Enable running HTTP server independently: `python src/http/server.py`
  - Add CLI help message with server information
  - Rationale: Provides convenient development and testing capabilities

### Phase 5: Dependencies and Requirements

- [ ] **5.1 Update Requirements File**
  - Add `fastapi>=0.115.0` for HTTP framework
  - Add `uvicorn[standard]>=0.32.0` for ASGI server
  - Add `python-multipart>=0.0.9` for request parsing
  - Note: protobuf 6.33.5 already present in `requirements.txt:18`
  - Rationale: FastAPI provides modern async HTTP framework, Uvicorn is production-ready ASGI server

- [ ] **5.2 Verify Protobuf Compatibility**
  - Confirm comm_pb2.py works with HTTP binary serialization
  - Test SerializeToString() and FromString() methods
  - Verify cross-platform binary compatibility (little/big endian)
  - Rationale: Ensures protobuf bindings generated by grpcio-tools work correctly for HTTP transport

### Phase 6: Testing and Validation

- [ ] **6.1 Create Manual Testing Script**
  - Create `src/http/test_client.py` for manual endpoint testing
  - Implement Python client using requests library with binary protobuf data
  - Test SearchBrainRegion endpoint with sample query
  - Test GetAllBrainRegions endpoint
  - Verify response deserialization and data integrity
  - Rationale: Provides development validation tool and usage examples for API consumers

- [ ] **6.2 Add Integration Testing Documentation**
  - Document binary protobuf request/response format in code comments
  - Add curl examples with --data-binary flag for command-line testing
  - Document Content-Type headers required for requests and responses
  - Create example JavaScript/TypeScript fetch code for frontend integration
  - Rationale: Ensures frontend developers can integrate with HTTP API correctly

## Verification Criteria

### Functional Requirements
- HTTP server starts successfully on port 5006 without errors
- POST /search-brain-region endpoint accepts binary protobuf requests and returns binary protobuf responses
- POST /get-all-brain-regions endpoint accepts binary protobuf requests and returns binary protobuf responses
- Response data matches format from gRPC service (same BrainRegionEntry structure)
- CORS headers present in responses for cross-origin requests
- Error responses contain appropriate status codes and error messages

### Data Integrity
- Binary protobuf serialization/deserialization works without data loss
- Timestamp conversions maintain millisecond precision (datetime → int64 → datetime)
- All 11 BrainRegionEntry fields populated correctly from database rows
- Null/empty field handling matches protobuf3 defaults
- Character encoding preserved for international characters in descriptions

### Performance
- Request/response latency comparable to gRPC service (< 100ms for simple queries)
- Server handles concurrent requests without blocking
- Binary protobuf payload size significantly smaller than JSON equivalent
- No memory leaks during repeated requests

### Integration
- HTTP client can successfully parse responses using comm_pb2 Python bindings
- Frontend can send binary protobuf requests from JavaScript/TypeScript
- HTTP and gRPC services return identical data for same queries
- Database connection pooling works correctly under load

## Potential Risks and Mitigations

### 1. **Binary Protobuf Browser Compatibility**
**Risk**: Web browsers may have difficulties sending/receiving binary protobuf data due to CORS preflight requests or Content-Type restrictions.

**Mitigation**: 
- Add comprehensive CORS configuration allowing application/x-protobuf and application/octet-stream content types
- Provide fallback JSON endpoints if binary protobuf proves problematic
- Test with protobuf.js library for JavaScript protobuf handling
- Document browser-specific requirements in API documentation

### 2. **Protobuf Version Mismatch**
**Risk**: Frontend protobuf library version may not match backend comm.proto definition, causing deserialization errors.

**Mitigation**:
- Version comm.proto file and include version in response headers
- Provide pre-compiled JavaScript protobuf definitions alongside API
- Implement backward compatibility checks in request parsing
- Add API version endpoint returning protobuf schema hash

### 3. **Port Conflicts and Docker Configuration**
**Risk**: Port 5006 may conflict with other services or require docker-compose.yml updates.

**Mitigation**:
- Make port configurable via environment variable
- Document port allocation in project README
- Update docker-compose.yml to expose HTTP port alongside gRPC
- Add health check endpoint for container orchestration

### 4. **Database Connection Exhaustion**
**Risk**: HTTP server may create too many concurrent database connections under high load.

**Mitigation**:
- Reuse existing repository layer connection handling from `db/schema.py:get_connection()`
- Consider implementing connection pooling if needed (psycopg2.pool)
- Add connection limit monitoring and alerting
- Configure Uvicorn worker count appropriately

### 5. **Error Information Leakage**
**Risk**: Detailed error messages may expose internal system information to clients.

**Mitigation**:
- Sanitize error messages before including in protobuf responses
- Log full error details server-side while returning generic messages to clients
- Use different error verbosity for development vs. production
- Implement structured error codes instead of free-text messages

### 6. **Request Size DoS Attacks**
**Risk**: Malicious clients may send extremely large protobuf payloads to exhaust server resources.

**Mitigation**:
- Configure FastAPI max request body size (e.g., 10MB limit)
- Add rate limiting middleware for API endpoints
- Monitor request size metrics and set up alerts
- Implement request timeout to prevent slow-loris attacks

## Alternative Approaches

### Alternative 1: gRPC-Web Gateway
**Description**: Instead of implementing a custom HTTP server, use gRPC-Web proxy (Envoy) to translate HTTP requests to gRPC.

**Trade-offs**:
- **Pros**: Reuses existing gRPC service without code changes; automatic protobuf handling; battle-tested proxy
- **Cons**: Additional deployment complexity (Envoy sidecar); limited customization of HTTP behavior; requires gRPC-Web client library
- **Recommendation**: Consider if infrastructure team already operates Envoy or similar proxies

### Alternative 2: JSON-over-HTTP with Protobuf Schema Validation
**Description**: Use JSON request/response format while validating against protobuf schema definitions.

**Trade-offs**:
- **Pros**: Easier frontend development; human-readable payloads; broader tooling support
- **Cons**: Larger payload sizes; slower serialization; loses protobuf efficiency benefits; requires JSON<->Protobuf conversion
- **Recommendation**: Provide as complementary endpoints if browser binary protobuf proves difficult

### Alternative 3: GraphQL API with Protobuf Backend
**Description**: Implement GraphQL layer that queries gRPC backend and returns JSON to frontend.

**Trade-offs**:
- **Pros**: Flexible querying; reduced over-fetching; excellent frontend DX with tooling
- **Cons**: Significant additional complexity; GraphQL schema maintenance; overkill for simple CRUD operations
- **Recommendation**: Consider only if building complex query patterns or multi-resource APIs

### Alternative 4: REST API with JSON and Binary Endpoints
**Description**: Provide dual endpoints supporting both JSON (`/api/v1/search`) and binary protobuf (`/api/v1/search.pb`) formats.

**Trade-offs**:
- **Pros**: Maximum flexibility; progressive migration path; supports diverse clients
- **Cons**: Doubled endpoint maintenance; potential schema drift between formats; API surface complexity
- **Recommendation**: Implement if supporting heterogeneous client ecosystem (web + mobile + IoT)

## Implementation Notes

### Technology Stack Justification
- **FastAPI**: Chosen for async support, automatic OpenAPI docs, modern Python type hints, and excellent binary data handling
- **Uvicorn**: Industry-standard ASGI server with high performance and good production stability
- **Existing Protobuf**: Reuses `comm_pb2.py` generated by grpcio-tools, avoiding duplicate schema definitions

### Architectural Consistency
The implementation follows established patterns from the codebase:
- Module organization mirrors `src/grpc/` structure
- Repository pattern reuse from `db/repository.py`
- Configuration via environment variables (python-dotenv pattern)
- Error handling matches gRPC service style
- Timestamp handling preserves millisecond precision from `server.py:24,31`

### Frontend Integration Path
Frontend developers should:
1. Use protobuf.js or similar library to load comm.proto definitions
2. Serialize requests: `SearchBrainRegionRequest.encode({query: "hippocampus"}).finish()`
3. Send via fetch with Content-Type: application/x-protobuf
4. Deserialize responses: `SearchBrainRegionResponse.decode(new Uint8Array(response))`
5. Handle status field to detect errors vs. successful results

### Deployment Considerations
- Add HTTP service to docker-compose.yml with port mapping 5006:5006
- Configure reverse proxy (nginx/traefik) to route /api/grpc → gRPC, /api/http → HTTP
- Set up monitoring for both gRPC and HTTP endpoints
- Document migration path for clients to adopt HTTP if desired

---

**Next Steps**: Proceed with implementation starting from Phase 1, creating the HTTP module structure and FastAPI server foundation. All implementation tasks use checkbox format for progress tracking.
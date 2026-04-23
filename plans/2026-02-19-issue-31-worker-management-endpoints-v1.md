# Issue #31: Add Worker Management API Endpoints to Orch Service

## Objective

Add worker management endpoints to the orch service that forward requests to the fetcher-be service, enabling frontend applications to manage workers through the orch API instead of directly accessing fetcher-be.

**Current State**: Worker management endpoints exist only in fetcher-be at `/fetcher-be/api/queue/workers/*`. Frontend applications must interact with multiple services.

**Desired State**: Orch service exposes worker management endpoints at `/orch/api/workers/*` that proxy requests to fetcher-be, providing a unified API surface.

## Problem Analysis

### Architecture Context

The CortexMap application uses a microservices architecture with three main services:

1. **Orch**: Orchestration layer - primary API gateway for frontend
2. **Fetcher-be**: Document fetching and queue management - owns worker lifecycle
3. **Brainatlas-be**: LLM-based summary generation

**Current Frontend Integration**:
- Primary API: `http://orch:3000/orch/api/*`
- Direct fetcher access needed for worker management (architectural inconsistency)

**Existing Orch → Fetcher Communication**:
- Already proxies worker allocation internally via `ensure_workers_allocated()` method
- Uses `OrchHttpClient` for HTTP communication
- Has established patterns for forwarding requests

### Existing Worker Endpoints in Fetcher-be

| Endpoint | Method | Purpose | Request Type | Response Type |
|----------|--------|---------|--------------|---------------|
| `/fetcher-be/api/queue/workers/status` | GET | Get worker status and statistics | None | `WorkerStatusResponse` |
| `/fetcher-be/api/queue/workers/allocate` | POST | Start worker processes | `AllocateWorkersRequest` | `AllocateWorkersResponse` |
| `/fetcher-be/api/queue/workers/stop` | POST | Stop workers gracefully | `StopWorkersRequest` | `StopWorkersResponse` |

**Reference Locations**:
- Routing: `fetcher-be/crates/cortexmap-be/src/server.rs:96-100`
- Handlers: `fetcher-be/crates/cortexmap-be/src/server.rs:415-478`
- Proto definitions: `proto/app/queue.proto:31-52,186-234`

## Implementation Plan

### Phase 1: Define API Contracts

- [ ] **1.1** Add worker management response types to `orch/crates/domain/src/lib.rs`
  - Create `WorkerStatus` struct mirroring fetcher's `WorkerInfo`
  - Create `WorkerAllocationResponse` with success flag and worker IDs
  - Create `WorkerStopResponse` with workers stopped count
  - Use existing proto types as reference but adapt for domain layer
  - **Rationale**: Domain layer should own its data structures independent of external service formats

- [ ] **1.2** Add worker management request types to `orch/crates/domain/src/lib.rs`
  - Create `AllocateWorkersRequest` with worker_count, task_timeout_secs, max_retry_attempts
  - Create `StopWorkersRequest` with optional worker_ids list
  - Match fetcher's proto schema for seamless forwarding
  - **Rationale**: Type-safe request structures ensure API consistency and validation

### Phase 2: Extend Service Layer Traits

- [ ] **2.1** Add worker management methods to `WorkerManagement` trait in `orch/crates/app/src/services.rs`
  - Add `async fn get_worker_status() -> Result<Vec<WorkerStatus>, Self::Error>`
  - Add `async fn allocate_workers(req: AllocateWorkersRequest) -> Result<WorkerAllocationResponse, Self::Error>`
  - Add `async fn stop_workers(req: StopWorkersRequest) -> Result<WorkerStopResponse, Self::Error>`
  - **Rationale**: Trait-based architecture requires extending service contracts before implementation

- [ ] **2.2** Update `Services` umbrella trait to include `WorkerManagement`
  - Add `WorkerManagement` as a supertrait bound in `orch/crates/app/src/services.rs:122-142`
  - Ensure all service implementations inherit worker management capability
  - **Rationale**: Maintains consistent service composition pattern used throughout orch

### Phase 3: Implement HTTP Forwarding in Services Layer

- [ ] **3.1** Implement `get_worker_status()` in `orch/crates/services/src/batch_orchestration.rs`
  - Resolve fetcher URL using existing `get_fetcher_url()` method
  - Call `GET {fetcher_url}/fetcher-be/api/queue/workers/status`
  - Transform `WorkerStatusResponse` proto type to domain `WorkerStatus` list
  - Handle HTTP errors and map to `ServiceError`
  - **Rationale**: Reuses existing fetcher communication patterns and error handling

- [ ] **3.2** Implement `allocate_workers()` in `orch/crates/services/src/batch_orchestration.rs`
  - Validate request parameters (worker_count > 0, timeout > 0)
  - Resolve fetcher URL
  - Call `POST {fetcher_url}/fetcher-be/api/queue/workers/allocate` with request body
  - Transform response from proto to domain type
  - Log allocation events at INFO level with worker count and IDs
  - **Rationale**: Provides visibility into worker allocation operations for debugging

- [ ] **3.3** Implement `stop_workers()` in `orch/crates/services/src/batch_orchestration.rs`
  - Resolve fetcher URL
  - Call `POST {fetcher_url}/fetcher-be/api/queue/workers/stop` with request body
  - Transform response from proto to domain type
  - Log worker stop operations with count at INFO level
  - **Rationale**: Ensures graceful worker shutdown is tracked in orch logs

- [ ] **3.4** Refactor existing `get_worker_status_internal()` to use new trait method
  - Update `ensure_workers_allocated()` at `batch_orchestration.rs:209-217` to call new trait method
  - Remove duplicate implementation
  - **Rationale**: Eliminates code duplication and ensures consistency

### Phase 4: Add API Layer Endpoints

- [ ] **4.1** Define API trait methods in `orch/crates/api/src/api.rs`
  - Add `async fn get_worker_status() -> Result<WorkerStatusApiResponse>`
  - Add `async fn allocate_workers(req: AllocateWorkersApiRequest) -> Result<WorkerAllocationApiResponse>`
  - Add `async fn stop_workers(req: StopWorkersApiRequest) -> Result<WorkerStopApiResponse>`
  - **Rationale**: API layer defines HTTP-specific request/response types separate from domain

- [ ] **4.2** Implement API methods in `orch/crates/api/src/orch_api.rs`
  - For each method, call corresponding service layer method
  - Transform domain types to API response types
  - Apply appropriate error mapping to HTTP status codes
  - **Rationale**: Thin API implementation delegates to service layer following established pattern

### Phase 5: Add HTTP Routes and Handlers

- [ ] **5.1** Create handler functions in `orch/crates/server/src/handlers.rs`
  - Create `get_worker_status_handler(State(api)) -> Json<WorkerStatusApiResponse>`
  - Create `allocate_workers_handler(State(api), Json(req)) -> Result<Json<WorkerAllocationApiResponse>, ApiError>`
  - Create `stop_workers_handler(State(api), Json(req)) -> Result<Json<WorkerStopApiResponse>, ApiError>`
  - Follow existing handler pattern from `get_config_handler` and similar endpoints
  - **Rationale**: Consistent handler structure makes codebase more maintainable

- [ ] **5.2** Register routes in `orch/crates/server/src/server.rs`
  - Add `GET /api/workers/status` route pointing to `get_worker_status_handler`
  - Add `POST /api/workers/allocate` route pointing to `allocate_workers_handler`
  - Add `POST /api/workers/stop` route pointing to `stop_workers_handler`
  - Insert routes after `/api/config` routes around line 77
  - **Rationale**: Groups related worker management endpoints together in routing table

### Phase 6: Error Handling Enhancement

- [ ] **6.1** Add worker-specific error variants if needed
  - Review if existing `ServiceError` and `ApiError` types handle worker errors adequately
  - Add variants like `WorkerAllocationFailed` or `WorkerNotFound` if required
  - Map fetcher HTTP errors to appropriate orch error types
  - **Rationale**: Clear error messages improve API usability and debugging

- [ ] **6.2** Add error context for fetcher communication failures
  - Wrap HTTP client errors with context about which worker operation failed
  - Include fetcher URL in error messages for troubleshooting
  - **Rationale**: Helps diagnose service communication issues in production

### Phase 7: Testing and Validation

- [ ] **7.1** Create integration test for worker status endpoint
  - Test GET `/orch/api/workers/status` returns worker information
  - Verify response schema matches expected structure
  - Test behavior when fetcher is unavailable
  - **Rationale**: Ensures endpoint works end-to-end and handles failure cases

- [ ] **7.2** Create integration test for worker allocation
  - Test POST `/orch/api/workers/allocate` with valid request
  - Verify workers are actually allocated in fetcher-be
  - Test validation of invalid parameters (zero workers, negative timeout)
  - **Rationale**: Validates worker lifecycle management through orch API

- [ ] **7.3** Create integration test for worker stop
  - Test POST `/orch/api/workers/stop` with specific worker IDs
  - Test stop all workers with empty list
  - Verify workers are stopped in fetcher-be
  - **Rationale**: Ensures graceful shutdown operations work correctly

- [ ] **7.4** Test error handling scenarios
  - Fetcher service down/unreachable
  - Invalid fetcher URL configuration
  - Malformed request bodies
  - Timeout scenarios
  - **Rationale**: Robust error handling is critical for production reliability

### Phase 8: Documentation and Configuration

- [ ] **8.1** Update API documentation
  - Document new endpoints in README or API docs
  - Include request/response examples
  - Document error codes and meanings
  - **Rationale**: API documentation helps frontend developers integrate correctly

- [ ] **8.2** Verify environment configuration
  - Ensure `FETCHER_HTTP_ADDR` is properly documented
  - Verify URL normalization handles all edge cases
  - Test with various fetcher URL formats
  - **Rationale**: Configuration issues are common source of deployment problems

- [ ] **8.3** Add logging and observability
  - Log all worker management operations with request context
  - Include correlation IDs if available
  - Add metrics for worker allocation/stop operations if metrics system exists
  - **Rationale**: Observability is essential for production monitoring and debugging

## Verification Criteria

### Functional Requirements

- [ ] GET `/orch/api/workers/status` returns list of workers with status, task counts, and uptime
- [ ] POST `/orch/api/workers/allocate` successfully starts workers and returns worker IDs
- [ ] POST `/orch/api/workers/stop` successfully stops workers (all or specific IDs)
- [ ] All endpoints properly forward requests to fetcher-be and transform responses
- [ ] Error responses include meaningful messages and appropriate HTTP status codes

### Non-Functional Requirements

- [ ] Response times are comparable to direct fetcher-be calls (minimal proxy overhead)
- [ ] All endpoints handle fetcher-be unavailability gracefully with clear error messages
- [ ] Concurrent requests to worker endpoints are handled correctly
- [ ] No code duplication - reuses existing HTTP client and error handling infrastructure
- [ ] Maintains architectural consistency with existing orch endpoints

### Integration Requirements

- [ ] Existing orch endpoints continue to function correctly
- [ ] Internal `ensure_workers_allocated()` uses new trait methods (no duplicate code)
- [ ] Frontend can manage workers entirely through orch API without direct fetcher access
- [ ] CORS configuration allows frontend to call new endpoints

## Potential Risks and Mitigations

### 1. **Service Communication Failures**
**Risk**: Fetcher-be unavailable or unreachable when orch tries to forward requests
**Mitigation**:
- Implement retry logic with exponential backoff for transient failures
- Return clear error messages indicating fetcher is down
- Consider health check endpoint that verifies fetcher connectivity
- Add timeout configuration for HTTP calls

### 2. **Type Mismatch Between Services**
**Risk**: Fetcher-be changes worker API schema breaking orch forwarding
**Mitigation**:
- Version API endpoints if schema changes are expected
- Add integration tests that catch schema mismatches
- Use shared proto definitions as source of truth
- Consider contract testing between services

### 3. **Performance Overhead**
**Risk**: Adding proxy layer introduces latency
**Mitigation**:
- Keep transformation logic minimal
- Avoid unnecessary data copying
- Monitor response times and set SLAs
- Consider caching worker status if polled frequently

### 4. **Inconsistent State**
**Risk**: Workers allocated through orch but state not reflected in orch database
**Mitigation**:
- Current design is stateless forwarding - orch doesn't track workers
- Document that worker state lives in fetcher-be
- Consider adding worker count to orch metrics/stats if needed

### 5. **Security Concerns**
**Risk**: Exposing worker management to frontend could allow resource exhaustion
**Mitigation**:
- Add authentication/authorization if not already present
- Implement rate limiting on worker allocation endpoint
- Set maximum worker count limits
- Log all worker management operations for audit trail

### 6. **Configuration Issues**
**Risk**: Incorrect fetcher URL configuration breaks worker management
**Mitigation**:
- Validate fetcher URL on startup
- Add health check that verifies fetcher connectivity
- Document required environment variables clearly
- Provide meaningful error when fetcher URL is misconfigured

## Alternative Approaches

### Alternative 1: Direct Frontend → Fetcher Communication
**Description**: Keep worker management in fetcher-be, have frontend call fetcher directly

**Pros**:
- No code changes needed in orch
- Simpler architecture
- Lower latency (no proxy hop)

**Cons**:
- Frontend must know about multiple services
- CORS configuration more complex
- Violates single API gateway pattern
- Harder to add cross-cutting concerns (auth, logging, rate limiting)

**Recommendation**: Not recommended - breaks architectural pattern of orch as primary API

### Alternative 2: Event-Driven Worker Management
**Description**: Orch publishes worker allocation events, fetcher subscribes and responds

**Pros**:
- Loose coupling between services
- Better scalability
- Natural fit for distributed systems

**Cons**:
- Adds complexity (message broker required)
- Harder to debug
- Synchronous frontend requests become asynchronous
- Overkill for simple forwarding use case

**Recommendation**: Not recommended for this use case - adds unnecessary complexity

### Alternative 3: Orch Owns Worker Management
**Description**: Move worker management from fetcher to orch, have orch control lifecycle

**Pros**:
- True unified API
- No forwarding needed
- Simpler for frontend

**Cons**:
- Violates separation of concerns (fetcher should own its workers)
- Requires significant refactoring
- Couples orch to fetcher implementation details
- Workers are fetcher-specific execution units

**Recommendation**: Not recommended - violates service boundaries

### Alternative 4: GraphQL Federation
**Description**: Use GraphQL with federated schemas across services

**Pros**:
- Unified query language
- Clients can request exactly what they need
- Built-in schema stitching

**Cons**:
- Complete API redesign required
- Adds GraphQL dependency and complexity
- Existing REST clients would break
- Overkill for adding three endpoints

**Recommendation**: Not recommended - too invasive for this requirement

## Implementation Assumptions

1. **Fetcher API Stability**: The fetcher-be worker API schema is stable and won't change during implementation
2. **Environment Configuration**: The `FETCHER_HTTP_ADDR` environment variable is correctly set in all deployment environments
3. **Error Handling Pattern**: Existing orch error handling patterns are sufficient for worker management errors
4. **Authentication**: Worker management endpoints will use the same authentication/authorization as other orch endpoints (if any)
5. **No Worker State in Orch**: Orch remains stateless regarding workers - all worker state lives in fetcher-be
6. **HTTP-only Communication**: gRPC is not required - HTTP/JSON forwarding is acceptable
7. **Synchronous Operations**: Worker allocation/stop operations complete synchronously (no long-polling or webhooks needed)
8. **Frontend Integration**: Frontend team will update their code to use new orch endpoints instead of direct fetcher calls

## Success Metrics

- All three worker management endpoints functional and tested
- Zero code duplication - internal worker allocation uses same code path
- Frontend can perform complete worker lifecycle management through orch API
- Error handling provides clear, actionable error messages
- Response times within 100ms of direct fetcher calls (excluding network latency)
- Integration tests achieve >90% code coverage of new functionality

## Dependencies

- **Proto definitions**: `proto/app/queue.proto` must remain stable or changes must be coordinated
- **Fetcher availability**: Fetcher-be service must be running for integration tests
- **HTTP client**: Existing `OrchHttpClient` in `orch/crates/infra/src/http.rs`
- **Service traits**: Pattern established in `orch/crates/app/src/services.rs`
- **Axum routing**: Framework used in `orch/crates/server/src/server.rs`

## Timeline Estimate

**Note**: This is a strategic plan - actual implementation will be performed by authorized agents

- **Phase 1-2** (Domain & Traits): ~2-3 hours - Define types and contracts
- **Phase 3** (Service Implementation): ~3-4 hours - HTTP forwarding logic
- **Phase 4-5** (API & Handlers): ~2-3 hours - Routing and handlers
- **Phase 6** (Error Handling): ~1-2 hours - Error mapping and context
- **Phase 7** (Testing): ~4-6 hours - Comprehensive integration tests
- **Phase 8** (Documentation): ~1-2 hours - API docs and configuration

**Total Estimated Effort**: 13-20 hours of development time

## Notes

- This implementation follows the established orch service patterns for consistency
- The proxy approach maintains clean service boundaries while providing frontend convenience
- Worker state remains in fetcher-be - orch is a pass-through layer only
- Existing internal worker allocation will be refactored to use the new trait methods
- All new code should follow Rust best practices and existing code style

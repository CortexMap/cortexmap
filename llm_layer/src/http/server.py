"""
HTTP server with binary protobuf support for Brain Atlas service.

This server provides HTTP endpoints that accept and return binary protobuf data,
mirroring the gRPC service functionality while being accessible to web clients.
"""

import sys
import os
import logging
from pathlib import Path
from typing import Dict, Any
from contextlib import asynccontextmanager

# Add parent directories to path to import protobuf bindings and db
sys.path.append(str(Path(__file__).parent.parent))
sys.path.append(str(Path(__file__).parent.parent / "grpc"))

from fastapi import FastAPI, Request, Response, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse
from dotenv import load_dotenv
from google.protobuf.message import DecodeError

import comm_pb2
from db.repository import search_by_quezry, get_all_responses

# Load environment variables
load_dotenv()

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)

# Lifespan context manager for startup and shutdown events
@asynccontextmanager
async def lifespan(app: FastAPI):
    """Handle startup and shutdown events."""
    # Startup
    host = os.getenv("HTTP_HOST", "0.0.0.0")
    port = os.getenv("HTTP_PORT", "5006")
    logger.info(f"Brain Atlas HTTP server starting on {host}:{port}")
    logger.info("Endpoints: /search-brain-region, /get-all-brain-regions")
    logger.info("Content-Type: application/x-protobuf")
    
    yield
    
    # Shutdown
    logger.info("Brain Atlas HTTP server shutting down")

# Create FastAPI application
app = FastAPI(
    title="Brain Atlas HTTP API",
    description="HTTP API with binary protobuf support for brain region queries",
    version="1.0.0",
    lifespan=lifespan
)

# Configure CORS
CORS_ORIGINS = os.getenv("CORS_ORIGINS", "*").split(",")
app.add_middleware(
    CORSMiddleware,
    allow_origins=CORS_ORIGINS,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
    expose_headers=["Content-Type"],
)


# ============================================================================
# Helper Functions
# ============================================================================

def row_to_brain_region_entry(row: Dict[str, Any]) -> comm_pb2.BrainRegionEntry:
    """
    Transform a database row dictionary to a protobuf BrainRegionEntry.
    
    Args:
        row: Dictionary containing database row data
        
    Returns:
        BrainRegionEntry protobuf message
    """
    return comm_pb2.BrainRegionEntry(
        id=row['id'],
        query=row['query'],
        query_timestamp=int(row['query_timestamp'].timestamp() * 1000),
        region_name=row['region_name'],
        hemisphere=row['hemisphere'],
        lobe=row['lobe'],
        anatomical_region=row['anatomical_region'],
        function_description=row['function_description'],
        disease_description=row['disease_description'],
        created_at=int(row['created_at'].timestamp() * 1000),
        updated_at=int(row['updated_at'].timestamp() * 1000),
    )


def deserialize_request(body: bytes, message_class):
    """
    Deserialize binary protobuf request.
    
    Args:
        body: Raw binary request body
        message_class: Protobuf message class to deserialize into
        
    Returns:
        Deserialized protobuf message
        
    Raises:
        HTTPException: If deserialization fails
    """
    try:
        request = message_class()
        request.ParseFromString(body)
        return request
    except DecodeError as e:
        logger.error(f"Protobuf deserialization error: {e}")
        raise HTTPException(
            status_code=400,
            detail=f"Invalid protobuf data: {str(e)}"
        )
    except Exception as e:
        logger.error(f"Unexpected deserialization error: {e}")
        raise HTTPException(
            status_code=400,
            detail=f"Failed to parse request: {str(e)}"
        )


def serialize_response(response_object) -> bytes:
    """
    Serialize protobuf response to binary format.
    
    Args:
        response_object: Protobuf message to serialize
        
    Returns:
        Binary serialized protobuf data
    """
    try:
        return response_object.SerializeToString()
    except Exception as e:
        logger.error(f"Protobuf serialization error: {e}")
        raise HTTPException(
            status_code=500,
            detail=f"Failed to serialize response: {str(e)}"
        )


# ============================================================================
# Middleware for Content-Type Validation
# ============================================================================

@app.middleware("http")
async def validate_content_type(request: Request, call_next):
    """
    Validate Content-Type header for protobuf endpoints.
    """
    # Skip validation for root and health endpoints
    if request.url.path in ["/", "/health"]:
        return await call_next(request)
    
    # For POST requests, validate Content-Type
    if request.method == "POST":
        content_type = request.headers.get("content-type", "")
        valid_types = ["application/x-protobuf", "application/octet-stream"]
        
        if not any(ct in content_type.lower() for ct in valid_types):
            return JSONResponse(
                status_code=415,
                content={
                    "error": "Unsupported Media Type",
                    "detail": f"Content-Type must be one of: {', '.join(valid_types)}",
                    "received": content_type
                }
            )
    
    response = await call_next(request)
    return response


# ============================================================================
# HTTP Endpoints
# ============================================================================

@app.get("/")
async def root():
    """Root endpoint with API information."""
    return {
        "service": "Brain Atlas HTTP API",
        "version": "1.0.0",
        "endpoints": {
            "/search-brain-region": "POST - Search for brain regions (binary protobuf)",
            "/get-all-brain-regions": "POST - Get all brain regions (binary protobuf)",
            "/health": "GET - Health check"
        },
        "content_type": "application/x-protobuf or application/octet-stream",
        "documentation": "/docs"
    }


@app.get("/health")
async def health_check():
    """Health check endpoint."""
    return {"status": "healthy", "service": "brain-atlas-http"}


@app.post("/search-brain-region")
async def search_brain_region(request: Request):
    """
    Search for brain regions by query.
    
    Accepts binary protobuf SearchBrainRegionRequest and returns
    binary protobuf SearchBrainRegionResponse.
    
    Content-Type: application/x-protobuf or application/octet-stream
    """
    try:
        # Read and deserialize request
        body = await request.body()
        logger.info(f"Received search request, body size: {len(body)} bytes")
        
        pb_request = deserialize_request(body, comm_pb2.SearchBrainRegionRequest)
        logger.info(f"Search query: {pb_request.query}")
        
        # Query database
        results = search_by_query(pb_request.query)
        logger.info(f"Found {len(results)} results")
        
        # Transform results to protobuf entries
        entries = [row_to_brain_region_entry(row) for row in results]
        
        # Create response
        status = "success" if entries else "not_found"
        pb_response = comm_pb2.SearchBrainRegionResponse(
            entries=entries,
            status=status,
            error_message="" if entries else "No entries found"
        )
        
        # Serialize and return
        response_data = serialize_response(pb_response)
        logger.info(f"Returning response, size: {len(response_data)} bytes")
        
        return Response(
            content=response_data,
            media_type="application/x-protobuf"
        )
        
    except HTTPException:
        raise
    except Exception as e:
        logger.error(f"Error in search_brain_region: {e}", exc_info=True)
        
        # Return error as protobuf response
        error_response = comm_pb2.SearchBrainRegionResponse(
            entries=[],
            status="error",
            error_message=str(e)
        )
        response_data = serialize_response(error_response)
        
        return Response(
            content=response_data,
            media_type="application/x-protobuf",
            status_code=500
        )


@app.post("/get-all-brain-regions")
async def get_all_brain_regions(request: Request):
    """
    Retrieve all brain region entries.
    
    Accepts binary protobuf GetAllBrainRegionsRequest and returns
    binary protobuf GetAllBrainRegionsResponse.
    
    Content-Type: application/x-protobuf or application/octet-stream
    """
    try:
        # Read and deserialize request
        body = await request.body()
        logger.info(f"Received get-all request, body size: {len(body)} bytes")
        
        pb_request = deserialize_request(body, comm_pb2.GetAllBrainRegionsRequest)
        logger.info("Processing get all brain regions request")
        
        # Query database
        results = get_all_responses()
        logger.info(f"Retrieved {len(results)} total entries")
        
        # Transform results to protobuf entries
        entries = [row_to_brain_region_entry(row) for row in results]
        
        # Create response
        pb_response = comm_pb2.GetAllBrainRegionsResponse(
            entries=entries,
            total_count=len(entries),
            status="success",
            error_message=""
        )
        
        # Serialize and return
        response_data = serialize_response(pb_response)
        logger.info(f"Returning response, size: {len(response_data)} bytes")
        
        return Response(
            content=response_data,
            media_type="application/x-protobuf"
        )
        
    except HTTPException:
        raise
    except Exception as e:
        logger.error(f"Error in get_all_brain_regions: {e}", exc_info=True)
        
        # Return error as protobuf response
        error_response = comm_pb2.GetAllBrainRegionsResponse(
            entries=[],
            total_count=0,
            status="error",
            error_message=str(e)
        )
        response_data = serialize_response(error_response)
        
        return Response(
            content=response_data,
            media_type="application/x-protobuf",
            status_code=500
        )


# ============================================================================
# Server Startup
# ============================================================================

def serve():
    """
    Start the HTTP server with Uvicorn.
    
    Configuration via environment variables:
    - HTTP_HOST: Host to bind to (default: 0.0.0.0)
    - HTTP_PORT: Port to listen on (default: 5006)
    - CORS_ORIGINS: Comma-separated list of allowed origins (default: *)
    """
    import uvicorn
    
    host = os.getenv("HTTP_HOST", "0.0.0.0")
    port = int(os.getenv("HTTP_PORT", "5006"))
    
    logger.info("="*60)
    logger.info("Brain Atlas HTTP API with Binary Protobuf Support")
    logger.info("="*60)
    logger.info(f"Host: {host}")
    logger.info(f"Port: {port}")
    logger.info(f"CORS Origins: {CORS_ORIGINS}")
    logger.info(f"Documentation: http://{host}:{port}/docs")
    logger.info("="*60)
    
    uvicorn.run(
        app,
        host=host,
        port=port,
        log_level="info",
        access_log=True
    )


if __name__ == '__main__':
    serve()

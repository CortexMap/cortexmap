"""
HTTP layer for Brain Atlas service with binary protobuf support.

This module provides HTTP endpoints that accept and return binary protobuf data,
mirroring the functionality of the gRPC service but accessible via standard HTTP.
"""

from .server import app, serve

__all__ = ['app', 'serve']

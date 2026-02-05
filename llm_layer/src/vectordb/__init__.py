"""
VectorDB Module - A modular vector database creator for document embeddings.

This module provides functionality to create and manage Chroma vector databases
from markdown documents.
"""

from .vectordb import (
    VectorDBConfig,
    DocumentLoader,
    TextProcessor,
    VectorStore,
    VectorDBManager,
    generate_data_store
)

__all__ = [
    'VectorDBConfig',
    'DocumentLoader',
    'TextProcessor',
    'VectorStore',
    'VectorDBManager',
    'generate_data_store'
]
__version__ = '1.0.0'

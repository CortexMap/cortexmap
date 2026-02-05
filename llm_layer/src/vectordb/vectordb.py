"""
Vector Database Module

Provides modular functionality for creating and managing Chroma vector databases
from markdown documents using LangChain and Ollama embeddings.
"""

import os
import shutil
from typing import List, Optional, Tuple
from pathlib import Path

from langchain_ollama import OllamaEmbeddings
from langchain_text_splitters import RecursiveCharacterTextSplitter
from langchain_community.vectorstores import Chroma
from langchain_community.document_loaders import DirectoryLoader
from langchain_core.documents import Document
from dotenv import load_dotenv


class VectorDBConfig:
    """Configuration manager for vector database parameters."""
    
    def __init__(
        self,
        chroma_path: Optional[str] = None,
        data_path: Optional[str] = None,
        embedding_model: str = "nomic-embed-text",
        chunk_size: int = 300,
        chunk_overlap: int = 100
    ):
        """
        Initialize VectorDB configuration.
        
        Args:
            chroma_path: Path to Chroma database directory (defaults to env var CHROMA_DB_PATH)
            data_path: Path to markdown documents directory (defaults to env var MD_DATA_PATH)
            embedding_model: Name of the Ollama embedding model to use
            chunk_size: Size of text chunks for splitting
            chunk_overlap: Overlap size between chunks
        """
        load_dotenv()
        self.chroma_path = chroma_path or os.getenv('CHROMA_DB_PATH')
        self.data_path = data_path or os.getenv('MD_DATA_PATH')
        self.embedding_model = embedding_model
        self.chunk_size = chunk_size
        self.chunk_overlap = chunk_overlap
    
    def validate(self) -> Tuple[bool, Optional[str]]:
        """
        Validate that all required configuration parameters are present.
        
        Returns:
            Tuple of (is_valid, error_message)
        """
        if not self.chroma_path:
            return False, "CHROMA_DB_PATH not set in environment or config"
        
        if not self.data_path:
            return False, "MD_DATA_PATH not set in environment or config"
        
        if not os.path.exists(self.data_path):
            return False, f"Data path does not exist: {self.data_path}"
        
        return True, None
    
    def get_paths(self) -> Tuple[str, str]:
        """
        Get the configured paths.
        
        Returns:
            Tuple of (chroma_path, data_path)
        """
        return self.chroma_path, self.data_path


class DocumentLoader:
    """Handles loading documents from the filesystem."""
    
    def __init__(self, data_path: str):
        """
        Initialize document loader.
        
        Args:
            data_path: Path to directory containing documents
        """
        self.data_path = data_path
    
    def load_markdown_files(self, glob_pattern: str = "*.md") -> List[Document]:
        """
        Load all markdown files from the data directory.
        
        Args:
            glob_pattern: Pattern to match files (default: "*.md")
        
        Returns:
            List of loaded Document objects
        """
        try:
            loader = DirectoryLoader(self.data_path, glob=glob_pattern)
            documents = loader.load()
            return documents
        except Exception as e:
            raise Exception(f"Failed to load documents from {self.data_path}: {e}")
    
    def get_document_count(self) -> int:
        """
        Get count of markdown files in the data directory.
        
        Returns:
            Number of .md files
        """
        return len(list(Path(self.data_path).glob("*.md")))


class TextProcessor:
    """Handles text splitting and chunking operations."""
    
    def __init__(self, chunk_size: int = 300, chunk_overlap: int = 100):
        """
        Initialize text processor.
        
        Args:
            chunk_size: Size of text chunks
            chunk_overlap: Overlap size between chunks
        """
        self.chunk_size = chunk_size
        self.chunk_overlap = chunk_overlap
        self.text_splitter = RecursiveCharacterTextSplitter(
            chunk_size=chunk_size,
            chunk_overlap=chunk_overlap,
            length_function=len,
            add_start_index=True
        )
    
    def split_documents(
        self,
        documents: List[Document],
        verbose: bool = True,
        show_sample: bool = True
    ) -> List[Document]:
        """
        Split documents into smaller chunks.
        
        Args:
            documents: List of documents to split
            verbose: Print progress information
            show_sample: Show sample chunk content and metadata
        
        Returns:
            List of document chunks
        """
        chunks = self.text_splitter.split_documents(documents)
        
        if verbose:
            print(f"Split {len(documents)} documents into {len(chunks)} chunks.")
        
        if show_sample and len(chunks) > 10:
            print("\nSample chunk (index 10):")
            print(f"Content: {chunks[10].page_content}")
            print(f"Metadata: {chunks[10].metadata}")
        
        return chunks
    
    def get_chunk_stats(self, chunks: List[Document]) -> dict:
        """
        Get statistics about the chunks.
        
        Args:
            chunks: List of document chunks
        
        Returns:
            Dictionary with chunk statistics
        """
        if not chunks:
            return {"count": 0, "avg_length": 0, "min_length": 0, "max_length": 0}
        
        lengths = [len(chunk.page_content) for chunk in chunks]
        return {
            "count": len(chunks),
            "avg_length": sum(lengths) / len(lengths),
            "min_length": min(lengths),
            "max_length": max(lengths)
        }


class VectorStore:
    """Manages vector store operations with Chroma."""
    
    def __init__(self, chroma_path: str, embedding_model: str = "nomic-embed-text"):
        """
        Initialize vector store manager.
        
        Args:
            chroma_path: Path to Chroma database directory
            embedding_model: Name of the Ollama embedding model
        """
        self.chroma_path = chroma_path
        self.embedding_model = embedding_model
        self.embeddings = OllamaEmbeddings(model=embedding_model)
    
    def clear_database(self) -> bool:
        """
        Clear the existing Chroma database.
        
        Returns:
            True if database was cleared, False if it didn't exist
        """
        if os.path.exists(self.chroma_path):
            shutil.rmtree(self.chroma_path)
            return True
        return False
    
    def create_from_documents(
        self,
        chunks: List[Document],
        verbose: bool = True
    ) -> Chroma:
        """
        Create a new Chroma database from document chunks.
        
        Args:
            chunks: List of document chunks to embed
            verbose: Print progress information
        
        Returns:
            Chroma database instance
        """
        try:
            if verbose:
                print(f"Creating vector database with {len(chunks)} chunks...")
                print(f"Using embedding model: {self.embedding_model}")
            
            db = Chroma.from_documents(
                documents=chunks,
                embedding=self.embeddings,
                persist_directory=self.chroma_path,
            )
            
            db.persist()
            
            if verbose:
                print(f"✓ Saved {len(chunks)} chunks to {self.chroma_path}")
            
            return db
        
        except Exception as e:
            raise Exception(f"Failed to create vector database: {e}")
    
    def load_database(self) -> Optional[Chroma]:
        """
        Load an existing Chroma database.
        
        Returns:
            Chroma database instance or None if doesn't exist
        """
        if not os.path.exists(self.chroma_path):
            return None
        
        try:
            db = Chroma(
                persist_directory=self.chroma_path,
                embedding_function=self.embeddings
            )
            return db
        except Exception as e:
            raise Exception(f"Failed to load vector database: {e}")
    
    def database_exists(self) -> bool:
        """
        Check if the database exists.
        
        Returns:
            True if database exists, False otherwise
        """
        return os.path.exists(self.chroma_path)


class VectorDBManager:
    """High-level manager for vector database operations."""
    
    def __init__(self, config: Optional[VectorDBConfig] = None):
        """
        Initialize vector database manager.
        
        Args:
            config: Optional VectorDBConfig instance. If None, creates from environment.
        """
        self.config = config or VectorDBConfig()
        self._validate_config()
        
        self.document_loader = DocumentLoader(self.config.data_path)
        self.text_processor = TextProcessor(
            chunk_size=self.config.chunk_size,
            chunk_overlap=self.config.chunk_overlap
        )
        self.vector_store = VectorStore(
            chroma_path=self.config.chroma_path,
            embedding_model=self.config.embedding_model
        )
    
    def _validate_config(self) -> None:
        """Validate configuration and raise exception if invalid."""
        is_valid, error_msg = self.config.validate()
        if not is_valid:
            raise ValueError(error_msg)
    
    def generate_database(
        self,
        clear_existing: bool = True,
        verbose: bool = True,
        show_sample: bool = True
    ) -> Tuple[Chroma, dict]:
        """
        Generate vector database from markdown documents.
        
        Args:
            clear_existing: Whether to clear existing database before creating new one
            verbose: Print progress information
            show_sample: Show sample chunk during processing
        
        Returns:
            Tuple of (Chroma database instance, statistics dictionary)
        """
        try:
            # Clear existing database if requested
            if clear_existing:
                cleared = self.vector_store.clear_database()
                if verbose and cleared:
                    print(f"Cleared existing database at {self.config.chroma_path}")
            
            # Load documents
            if verbose:
                print(f"Loading documents from {self.config.data_path}...")
            documents = self.document_loader.load_markdown_files()
            
            if not documents:
                raise ValueError(f"No documents found in {self.config.data_path}")
            
            if verbose:
                print(f"Loaded {len(documents)} documents")
            
            # Split into chunks
            chunks = self.text_processor.split_documents(
                documents,
                verbose=verbose,
                show_sample=show_sample
            )
            
            # Create vector database
            db = self.vector_store.create_from_documents(chunks, verbose=verbose)
            
            # Gather statistics
            stats = {
                "documents_loaded": len(documents),
                "chunks_created": len(chunks),
                "chunk_stats": self.text_processor.get_chunk_stats(chunks),
                "database_path": self.config.chroma_path,
                "embedding_model": self.config.embedding_model
            }
            
            return db, stats
        
        except Exception as e:
            if verbose:
                print(f"✗ Error: {e}")
            raise
    
    def load_existing_database(self) -> Optional[Chroma]:
        """
        Load an existing vector database.
        
        Returns:
            Chroma database instance or None if doesn't exist
        """
        return self.vector_store.load_database()
    
    def database_exists(self) -> bool:
        """
        Check if database exists.
        
        Returns:
            True if database exists, False otherwise
        """
        return self.vector_store.database_exists()


def generate_data_store() -> None:
    """
    Convenience function to generate vector database using default configuration.
    
    This function maintains backward compatibility with the original script.
    """
    try:
        manager = VectorDBManager()
        manager.generate_database()
    except Exception as e:
        print(f"✗ Error: {e}")


if __name__ == "__main__":
    generate_data_store()

"""
Vector Database Query Module

Provides functionality to query the ChromaDB vector database for brain region information
and format responses using the LLM. This module reuses the VectorStore infrastructure
from src.vectordb.vectordb.py.
"""

import os
import sys
from pathlib import Path
from typing import List, Optional, Dict, Any
from dotenv import load_dotenv

# Add parent directory to path for imports
sys.path.append(str(Path(__file__).parent.parent.parent))

from langchain_core.documents import Document
import ollama

# Import shared VectorDB infrastructure
from vectordb.vectordb import VectorDBConfig, VectorStore, VectorDBManager

# Import the LLM response formatter
from query.llm import BrainRegion

load_dotenv()


class VectorDBQueryManager:
    """
    Manages querying the ChromaDB vector database for brain region information.
    
    This class leverages the VectorStore from vectordb.vectordb.py for database
    operations and adds query-specific functionality for brain region searches.
    """
    
    def __init__(
        self,
        chroma_path: Optional[str] = None,
        embedding_model: Optional[str] = None,
        top_k: int = 5
    ):
        """
        Initialize the VectorDB query manager.
        
        Args:
            chroma_path: Path to ChromaDB database (defaults to env CHROMA_DB_PATH)
            embedding_model: Embedding model name (defaults to nomic-embed-text)
            top_k: Number of top results to retrieve (default: 5)
        """
        self.top_k = top_k
        
        # Use VectorDBConfig for consistent configuration
        config = VectorDBConfig(
            chroma_path=chroma_path,
            embedding_model=embedding_model or "nomic-embed-text"
        )
        
        # Reuse VectorStore for database operations
        self.vector_store = VectorStore(
            chroma_path=config.chroma_path,
            embedding_model=config.embedding_model
        )
        
        # Load the database
        self.db = self._load_database()
        self.chroma_path = config.chroma_path
        self.embedding_model = config.embedding_model
    
    def _load_database(self):
        """
        Load the ChromaDB database using VectorStore.
        
        Returns:
            Chroma database instance
        
        Raises:
            FileNotFoundError: If database doesn't exist
            Exception: If database loading fails
        """
        if not self.vector_store.database_exists():
            raise FileNotFoundError(
                f"ChromaDB not found at {self.vector_store.chroma_path}. "
                f"Please run: python -m src.vectordb.vectordb"
            )
        
        db = self.vector_store.load_database()
        if db is None:
            raise Exception(f"Failed to load database at {self.vector_store.chroma_path}")
        
        return db
    
    def similarity_search(
        self,
        query: str,
        k: Optional[int] = None
    ) -> List[Document]:
        """
        Perform similarity search in the vector database.
        
        Args:
            query: Search query string
            k: Number of results to return (defaults to self.top_k)
        
        Returns:
            List of Document objects with relevant content
        """
        k = k or self.top_k
        
        try:
            results = self.db.similarity_search(query, k=k)
            return results
        except Exception as e:
            raise Exception(f"Similarity search failed: {e}")
    
    def similarity_search_with_score(
        self,
        query: str,
        k: Optional[int] = None
    ) -> List[tuple]:
        """
        Perform similarity search with relevance scores.
        
        Args:
            query: Search query string
            k: Number of results to return (defaults to self.top_k)
        
        Returns:
            List of (Document, score) tuples, where lower score = more similar
        """
        k = k or self.top_k
        
        try:
            results = self.db.similarity_search_with_score(query, k=k)
            return results
        except Exception as e:
            raise Exception(f"Similarity search with score failed: {e}")
    
    def get_relevant_context(
        self,
        query: str,
        k: Optional[int] = None,
        include_metadata: bool = True
    ) -> str:
        """
        Retrieve relevant context from vector database and format as string.
        
        Args:
            query: Search query string
            k: Number of results to retrieve
            include_metadata: Whether to include source metadata
        
        Returns:
            Formatted context string from retrieved documents
        """
        results = self.similarity_search(query, k=k)
        
        if not results:
            return ""
        
        context_parts = []
        for i, doc in enumerate(results, 1):
            context_parts.append(f"\n--- Context {i} ---")
            
            if include_metadata and doc.metadata:
                source = doc.metadata.get('source', 'Unknown')
                context_parts.append(f"Source: {source}")
            
            context_parts.append(doc.page_content)
        
        return "\n".join(context_parts)
    
    def query_with_context(
        self,
        region_name: str,
        k: Optional[int] = None,
        verbose: bool = False
    ) -> BrainRegion:
        """
        Query for brain region information using vector database context.
        
        This method:
        1. Searches ChromaDB for relevant context about the brain region
        2. Passes the context to the LLM for structured response generation
        3. Returns a validated BrainRegion object
        
        Args:
            region_name: Name of the brain region to query
            k: Number of context documents to retrieve (defaults to self.top_k)
            verbose: Print debug information
        
        Returns:
            BrainRegion object with structured information
        
        Raises:
            Exception: If vector search or LLM query fails
        """
        try:
            # Build search query
            search_query = f"brain region {region_name} anatomy function disease"
            
            if verbose:
                print(f"Searching ChromaDB for: {region_name}")
            
            # Retrieve relevant context from vector database
            context = self.get_relevant_context(search_query, k=k)
            
            if verbose:
                print(f"Retrieved {len(context)} characters of context")
                if context:
                    print(f"Context preview: {context[:200]}...")
            
            # Build prompt with context
            prompt = self._build_prompt_with_context(region_name, context)
            
            if verbose:
                print(f"Querying LLM with context...")
            
            # Query LLM with structured output
            response = ollama.chat(
                model='deepseek-r1:8b',
                messages=[{
                    'role': 'user',
                    'content': prompt
                }],
                format=BrainRegion.model_json_schema()
            )
            
            # Parse and validate response
            brain_region = BrainRegion.model_validate_json(response.message.content)
            
            if verbose:
                print(f"✓ Successfully generated response for: {brain_region.name}")
            
            return brain_region
            
        except Exception as e:
            if verbose:
                print(f"✗ Error during query: {e}")
            raise Exception(f"Failed to query with context for {region_name}: {e}")
    
    def _build_prompt_with_context(self, region_name: str, context: str) -> str:
        """
        Build a detailed prompt with retrieved context.
        
        Args:
            region_name: Name of the brain region
            context: Retrieved context from vector database
        
        Returns:
            Formatted prompt string
        """
        prompt = f"""You are an expert neuroscience knowledge base generator. Your task is to generate accurate, clinically relevant information about the {region_name}.

Use the following reference information to provide accurate details:

{context}

Based on the above context and your knowledge, provide comprehensive information about the {region_name} including:
1. Its precise anatomical location (hemisphere, lobe, anatomical region)
2. A detailed 150-250 word description of its functions
3. A detailed 150-250 word description of diseases and disorders associated with this region

Ensure your response is medically accurate and based on the provided context."""
        
        return prompt
    
    def get_database_stats(self) -> Dict[str, Any]:
        """
        Get statistics about the loaded database.
        
        Returns:
            Dictionary with database statistics
        """
        try:
            # Get collection count
            collection = self.db._collection
            count = collection.count()
            
            return {
                "database_path": self.chroma_path,
                "embedding_model": self.embedding_model,
                "total_documents": count,
                "top_k_default": self.top_k
            }
        except Exception as e:
            return {
                "error": str(e),
                "database_path": self.chroma_path
            }


def query_brain_region_with_vector_db(
    region_name: str,
    chroma_path: Optional[str] = None,
    top_k: int = 5,
    verbose: bool = False
) -> BrainRegion:
    """
    Convenience function to query brain region information using vector database.
    
    Args:
        region_name: Name of the brain region to query
        chroma_path: Path to ChromaDB (defaults to CHROMA_DB_PATH env var)
        top_k: Number of context documents to retrieve
        verbose: Print debug information
    
    Returns:
        BrainRegion object with structured information
    
    Example:
        >>> brain_region = query_brain_region_with_vector_db("hippocampus", verbose=True)
        >>> print(brain_region.name)
        >>> print(brain_region.location.hemisphere)
        >>> print(brain_region.function_diseases.function_description)
    """
    manager = VectorDBQueryManager(chroma_path=chroma_path, top_k=top_k)
    return manager.query_with_context(region_name, verbose=verbose)


# Example usage and testing
if __name__ == "__main__":
    import argparse
    
    parser = argparse.ArgumentParser(description="Query brain region information using vector database")
    parser.add_argument("region_name", help="Name of the brain region to query")
    parser.add_argument("--chroma-path", help="Path to ChromaDB database")
    parser.add_argument("--top-k", type=int, default=5, help="Number of context documents to retrieve")
    parser.add_argument("--verbose", action="store_true", help="Print verbose output")
    
    args = parser.parse_args()
    
    try:
        print(f"Querying brain region: {args.region_name}")
        print("=" * 80)
        
        result = query_brain_region_with_vector_db(
            region_name=args.region_name,
            chroma_path=args.chroma_path,
            top_k=args.top_k,
            verbose=args.verbose
        )
        
        print(f"\n✓ Query successful!\n")
        print(f"Name: {result.name}")
        print(f"Hemisphere: {result.location.hemisphere.value}")
        print(f"Lobe: {result.location.lobe}")
        print(f"Anatomical Region: {result.location.anatomical_region}")
        print(f"\nFunction Description ({len(result.function_diseases.function_description)} chars):")
        print(result.function_diseases.function_description)
        print(f"\nDisease Description ({len(result.function_diseases.disease_description)} chars):")
        print(result.function_diseases.disease_description)
        
    except FileNotFoundError as e:
        print(f"✗ Error: {e}")
        print(f"\nTo create the vector database, run:")
        print(f"  python -m src.vectordb.vectordb")
    except Exception as e:
        print(f"✗ Error: {e}")
        import traceback
        traceback.print_exc()

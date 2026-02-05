#!/usr/bin/env python3
"""
Test script for Vector Database Query functionality.

This script tests the integration between ChromaDB vector search and LLM formatting.
"""

import sys
from pathlib import Path

# Add project root to path
sys.path.insert(0, str(Path(__file__).parent.parent))

from src.query.vectordb import VectorDBQueryManager, query_brain_region_with_vector_db


def test_basic_query():
    """Test basic query functionality."""
    print("="*80)
    print("TEST 1: Basic Query")
    print("="*80)
    
    try:
        result = query_brain_region_with_vector_db("hippocampus", verbose=True)
        print(f"\n✓ Success!")
        print(f"  Name: {result.name}")
        print(f"  Hemisphere: {result.location.hemisphere.value}")
        print(f"  Lobe: {result.location.lobe}")
        return True
    except FileNotFoundError as e:
        print(f"\n⚠️  ChromaDB not found: {e}")
        print(f"  Run: python3 -m src.vectordb.vectordb")
        return False
    except Exception as e:
        print(f"\n✗ Error: {e}")
        return False


def test_similarity_search():
    """Test similarity search without LLM."""
    print("\n" + "="*80)
    print("TEST 2: Similarity Search Only")
    print("="*80)
    
    try:
        manager = VectorDBQueryManager(top_k=3)
        docs = manager.similarity_search("amygdala anatomy function", k=3)
        
        print(f"\n✓ Found {len(docs)} relevant documents")
        for i, doc in enumerate(docs, 1):
            print(f"\nDocument {i}:")
            print(f"  Content preview: {doc.page_content[:150]}...")
            if doc.metadata:
                print(f"  Source: {doc.metadata.get('source', 'Unknown')}")
        return True
    except Exception as e:
        print(f"\n✗ Error: {e}")
        return False


def test_context_retrieval():
    """Test context retrieval and formatting."""
    print("\n" + "="*80)
    print("TEST 3: Context Retrieval")
    print("="*80)
    
    try:
        manager = VectorDBQueryManager(top_k=5)
        context = manager.get_relevant_context("prefrontal cortex executive function", k=3)
        
        print(f"\n✓ Retrieved context ({len(context)} characters)")
        print(f"Context preview:\n{context[:300]}...")
        return True
    except Exception as e:
        print(f"\n✗ Error: {e}")
        return False


def test_database_stats():
    """Test database statistics."""
    print("\n" + "="*80)
    print("TEST 4: Database Statistics")
    print("="*80)
    
    try:
        manager = VectorDBQueryManager()
        stats = manager.get_database_stats()
        
        print(f"\n✓ Database Stats:")
        for key, value in stats.items():
            print(f"  {key}: {value}")
        return True
    except Exception as e:
        print(f"\n✗ Error: {e}")
        return False


def main():
    """Run all tests."""
    print("\nVector Database Query Integration Tests")
    print("="*80)
    
    results = []
    
    # Test 1: Basic query (requires Ollama)
    results.append(("Basic Query", test_basic_query()))
    
    # Test 2: Similarity search (no Ollama needed)
    results.append(("Similarity Search", test_similarity_search()))
    
    # Test 3: Context retrieval
    results.append(("Context Retrieval", test_context_retrieval()))
    
    # Test 4: Database stats
    results.append(("Database Stats", test_database_stats()))
    
    # Summary
    print("\n" + "="*80)
    print("TEST SUMMARY")
    print("="*80)
    
    for test_name, passed in results:
        status = "✓ PASS" if passed else "✗ FAIL"
        print(f"{status}: {test_name}")
    
    total = len(results)
    passed = sum(1 for _, p in results if p)
    
    print(f"\nTotal: {passed}/{total} tests passed")
    
    if passed == total:
        print("🎉 All tests passed!")
        return 0
    else:
        print("⚠️  Some tests failed")
        return 1


if __name__ == "__main__":
    sys.exit(main())

"""
Example script demonstrating how to query the LLM and store responses in PostgreSQL.
"""

import sys
from pathlib import Path
from dotenv import load_dotenv

# Add parent directory to path
sys.path.append(str(Path(__file__).parent.parent))

from query.llm import brain_region_query
from db.repository import store_brain_region_response
from db.schema import init_database


def query_and_store(query: str):
    """
    Query the LLM about a brain region and store the response in the database.
    
    Args:
        query: The user's query about a brain region
        include_context: Whether to include context from markdown files
        model_name: Name of the LLM model to use
        
    Returns:
        Tuple of (BrainRegion object, database record ID)
    """
    print(f"\nQuerying LLM: {query}")
    print("-" * 80)
    
    # Query the LLM
    brain_region = brain_region_query(query)
    
    # Display the response
    print(f"\nLLM Response:")
    print(f"Name: {brain_region.name}")
    print(f"Hemisphere: {brain_region.location.hemisphere.value}")
    print(f"Lobe: {brain_region.location.lobe}")
    print(f"Anatomical Region: {brain_region.location.anatomical_region}")
    print(f"\nFunction: {brain_region.function_diseases.function_description[:200]}...")
    print(f"\nDisease: {brain_region.function_diseases.disease_description[:200]}...")
    
    # Store in database
    print("\nStoring response in database...")
    record_id = store_brain_region_response(
        query=query,
        brain_region=brain_region
    )
    
    if record_id:
        print(f"✓ Successfully stored with ID: {record_id}")
    else:
        print("✗ Failed to store response")
    
    print("-" * 80)
    
    return brain_region, record_id


if __name__ == "__main__":
    # Load environment variables
    load_dotenv()
    
    # Initialize database (creates table if it doesn't exist)
    print("Initializing database...")
    init_database()
    
    # Example queries
    queries = [
        "hippocampus.",
        "prefrontal cortex",
        "amygdala.",
    ]
    
    # Process each query
    for query in queries:
        try:
            brain_region, record_id = query_and_store(
                query=query,
            )
        except Exception as e:
            print(f"Error processing query '{query}': {e}")
            continue
    
    print("\n✓ All queries processed and stored!")

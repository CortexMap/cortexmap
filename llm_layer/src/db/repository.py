"""
Repository methods for storing and retrieving LLM responses from PostgreSQL.
"""

import psycopg2
from psycopg2.extras import RealDictCursor
from typing import Optional, List, Dict, Any
from datetime import datetime
import sys
from pathlib import Path

# Add parent directory to path to import queryllm
sys.path.append(str(Path(__file__).parent.parent))

from query.queryllm import BrainRegion
from db.schema import get_connection


def store_brain_region_response(
    query: str,
    brain_region: BrainRegion,
    model_name: str = 'deepseek-r1:8b',
    include_context: bool = False
) -> Optional[int]:
    """
    Store a BrainRegion response in the database.
    
    Args:
        query: The original user query
        brain_region: The BrainRegion object returned by the LLM
        model_name: Name of the LLM model used
        include_context: Whether context was included in the query
        
    Returns:
        The ID of the inserted record, or None if insertion failed
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor()
        
        insert_sql = """
        INSERT INTO brain_region_responses (
            query,
            region_name,
            hemisphere,
            lobe,
            anatomical_region,
            function_description,
            disease_description,
            model_name,
            include_context
        ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s)
        RETURNING id;
        """
        
        cursor.execute(insert_sql, (
            query,
            brain_region.name,
            brain_region.location.hemisphere.value,
            brain_region.location.lobe,
            brain_region.location.anatomical_region,
            brain_region.function_diseases.function_description,
            brain_region.function_diseases.disease_description,
            model_name,
            include_context
        ))
        
        record_id = cursor.fetchone()[0]
        conn.commit()
        
        print(f"Successfully stored brain region response with ID: {record_id}")
        return record_id
        
    except psycopg2.Error as e:
        print(f"Error storing brain region response: {e}")
        if conn:
            conn.rollback()
        return None
        
    finally:
        if conn:
            cursor.close()
            conn.close()


def get_brain_region_response_by_id(record_id: int) -> Optional[Dict[str, Any]]:
    """
    Retrieve a brain region response by its ID.
    
    Args:
        record_id: The ID of the record to retrieve
        
    Returns:
        Dictionary containing the record data, or None if not found
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor(cursor_factory=RealDictCursor)
        
        select_sql = """
        SELECT * FROM brain_region_responses
        WHERE id = %s;
        """
        
        cursor.execute(select_sql, (record_id,))
        result = cursor.fetchone()
        
        return dict(result) if result else None
        
    except psycopg2.Error as e:
        print(f"Error retrieving brain region response: {e}")
        return None
        
    finally:
        if conn:
            cursor.close()
            conn.close()


def get_brain_region_responses_by_name(region_name: str) -> List[Dict[str, Any]]:
    """
    Retrieve all responses for a specific brain region name.
    
    Args:
        region_name: Name of the brain region
        
    Returns:
        List of dictionaries containing matching records
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor(cursor_factory=RealDictCursor)
        
        select_sql = """
        SELECT * FROM brain_region_responses
        WHERE region_name ILIKE %s
        ORDER BY query_timestamp DESC;
        """
        
        cursor.execute(select_sql, (f"%{region_name}%",))
        results = cursor.fetchall()
        
        return [dict(row) for row in results]
        
    except psycopg2.Error as e:
        print(f"Error retrieving brain region responses by name: {e}")
        return []
        
    finally:
        if conn:
            cursor.close()
            conn.close()


def get_recent_responses(limit: int = 10) -> List[Dict[str, Any]]:
    """
    Retrieve the most recent brain region responses.
    
    Args:
        limit: Maximum number of records to retrieve
        
    Returns:
        List of dictionaries containing recent records
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor(cursor_factory=RealDictCursor)
        
        select_sql = """
        SELECT * FROM brain_region_responses
        ORDER BY query_timestamp DESC
        LIMIT %s;
        """
        
        cursor.execute(select_sql, (limit,))
        results = cursor.fetchall()
        
        return [dict(row) for row in results]
        
    except psycopg2.Error as e:
        print(f"Error retrieving recent responses: {e}")
        return []
        
    finally:
        if conn:
            cursor.close()
            conn.close()


def search_by_query(search_term: str) -> List[Dict[str, Any]]:
    """
    Search for responses by query text.
    
    Args:
        search_term: Term to search for in queries
        
    Returns:
        List of dictionaries containing matching records
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor(cursor_factory=RealDictCursor)
        
        select_sql = """
        SELECT * FROM brain_region_responses
        WHERE query ILIKE %s
        ORDER BY query_timestamp DESC;
        """
        
        cursor.execute(select_sql, (f"%{search_term}%",))
        results = cursor.fetchall()
        
        return [dict(row) for row in results]
        
    except psycopg2.Error as e:
        print(f"Error searching by query: {e}")
        return []
        
    finally:
        if conn:
            cursor.close()
            conn.close()


def get_responses_by_hemisphere(hemisphere: str) -> List[Dict[str, Any]]:
    """
    Retrieve all responses for a specific hemisphere.
    
    Args:
        hemisphere: Hemisphere to filter by ('Left', 'Right', or 'Bilateral')
        
    Returns:
        List of dictionaries containing matching records
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor(cursor_factory=RealDictCursor)
        
        select_sql = """
        SELECT * FROM brain_region_responses
        WHERE hemisphere = %s
        ORDER BY query_timestamp DESC;
        """
        
        cursor.execute(select_sql, (hemisphere,))
        results = cursor.fetchall()
        
        return [dict(row) for row in results]
        
    except psycopg2.Error as e:
        print(f"Error retrieving responses by hemisphere: {e}")
        return []
        
    finally:
        if conn:
            cursor.close()
            conn.close()


def delete_response(record_id: int) -> bool:
    """
    Delete a brain region response by its ID.
    
    Args:
        record_id: The ID of the record to delete
        
    Returns:
        True if deletion was successful, False otherwise
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor()
        
        delete_sql = """
        DELETE FROM brain_region_responses
        WHERE id = %s;
        """
        
        cursor.execute(delete_sql, (record_id,))
        rows_deleted = cursor.rowcount
        conn.commit()
        
        if rows_deleted > 0:
            print(f"Successfully deleted record with ID: {record_id}")
            return True
        else:
            print(f"No record found with ID: {record_id}")
            return False
        
    except psycopg2.Error as e:
        print(f"Error deleting response: {e}")
        if conn:
            conn.rollback()
        return False
        
    finally:
        if conn:
            cursor.close()
            conn.close()


def get_statistics() -> Optional[Dict[str, Any]]:
    """
    Get statistics about stored brain region responses.
    
    Returns:
        Dictionary containing statistics
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor(cursor_factory=RealDictCursor)
        
        stats_sql = """
        SELECT 
            COUNT(*) as total_responses,
            COUNT(DISTINCT region_name) as unique_regions,
            COUNT(DISTINCT hemisphere) as unique_hemispheres,
            MIN(query_timestamp) as earliest_query,
            MAX(query_timestamp) as latest_query
        FROM brain_region_responses;
        """
        
        cursor.execute(stats_sql)
        result = cursor.fetchone()
        
        return dict(result) if result else None
        
    except psycopg2.Error as e:
        print(f"Error retrieving statistics: {e}")
        return None
        
    finally:
        if conn:
            cursor.close()
            conn.close()


if __name__ == "__main__":
    # Example usage
    from dotenv import load_dotenv
    load_dotenv()
    
    # Get statistics
    stats = get_statistics()
    if stats:
        print("\nDatabase Statistics:")
        print(f"Total responses: {stats['total_responses']}")
        print(f"Unique regions: {stats['unique_regions']}")
        print(f"Unique hemispheres: {stats['unique_hemispheres']}")
        print(f"Earliest query: {stats['earliest_query']}")
        print(f"Latest query: {stats['latest_query']}")
    
    # Get recent responses
    print("\nRecent responses:")
    recent = get_recent_responses(limit=5)
    for response in recent:
        print(f"- {response['region_name']} ({response['query_timestamp']})")

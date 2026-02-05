"""
Database schema for storing LLM responses about brain regions.
Uses psycopg2 for PostgreSQL operations.
"""

import psycopg2
from psycopg2.extras import RealDictCursor
from typing import Optional
import os
from dotenv import load_dotenv

load_dotenv()

# Database connection parameters
DB_CONFIG = {
    'host': os.getenv('DB_HOST', 'localhost'),
    'port': os.getenv('DB_PORT', '5433'),
    'database': os.getenv('DB_NAME', 'llmlayer'),
    'user': os.getenv('DB_USER', 'llmlayer'),
    'password': os.getenv('DB_PASSWORD', 'llmlayer_dev')
}

# SQL Schema for brain_region_responses table
CREATE_TABLE_SQL = """
CREATE TABLE IF NOT EXISTS brain_region_responses (
    id SERIAL PRIMARY KEY,
    
    -- Query metadata
    query TEXT NOT NULL,
    query_timestamp TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    
    -- Brain region basic info
    region_name VARCHAR(255) NOT NULL,
    
    -- Location information
    hemisphere VARCHAR(20) NOT NULL CHECK (hemisphere IN ('Left', 'Right', 'Bilateral')),
    lobe VARCHAR(255) NOT NULL,
    anatomical_region TEXT NOT NULL,
    
    -- Function and disease descriptions
    function_description TEXT NOT NULL,
    disease_description TEXT NOT NULL,
    
    -- Response metadata
    model_name VARCHAR(100),
    include_context BOOLEAN DEFAULT FALSE,
    
    -- Timestamps
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Create index on region_name for faster lookups
CREATE INDEX IF NOT EXISTS idx_region_name ON brain_region_responses(region_name);

-- Create index on query_timestamp for time-based queries
CREATE INDEX IF NOT EXISTS idx_query_timestamp ON brain_region_responses(query_timestamp);

-- Create trigger to automatically update updated_at timestamp
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = CURRENT_TIMESTAMP;
    RETURN NEW;
END;
$$ language 'plpgsql';

DROP TRIGGER IF EXISTS update_brain_region_responses_updated_at ON brain_region_responses;

CREATE TRIGGER update_brain_region_responses_updated_at
    BEFORE UPDATE ON brain_region_responses
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
"""


def get_connection():
    """
    Create and return a database connection.
    
    Returns:
        psycopg2 connection object
    """
    try:
        conn = psycopg2.connect(**DB_CONFIG)
        return conn
    except psycopg2.Error as e:
        print(f"Error connecting to database: {e}")
        raise


def init_database():
    """
    Initialize the database by creating the table and indexes.
    Should be run once during setup.
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor()
        
        # Execute schema creation
        cursor.execute(CREATE_TABLE_SQL)
        conn.commit()
        
        print("Database schema initialized successfully.")
        return True
        
    except psycopg2.Error as e:
        print(f"Error initializing database: {e}")
        if conn:
            conn.rollback()
        return False
        
    finally:
        if conn:
            cursor.close()
            conn.close()


def drop_table():
    """
    Drop the brain_region_responses table. Use with caution!
    """
    conn = None
    try:
        conn = get_connection()
        cursor = conn.cursor()
        
        cursor.execute("DROP TABLE IF EXISTS brain_region_responses CASCADE;")
        conn.commit()
        
        print("Table dropped successfully.")
        return True
        
    except psycopg2.Error as e:
        print(f"Error dropping table: {e}")
        if conn:
            conn.rollback()
        return False
        
    finally:
        if conn:
            cursor.close()
            conn.close()


if __name__ == "__main__":
    # Initialize database when run directly
    init_database()

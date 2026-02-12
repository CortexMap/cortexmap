"""
Database schema for storing LLM responses about brain regions.
Uses psycopg2 for PostgreSQL operations.
"""

import psycopg2
import os
from psycopg2.extras import execute_values
import csv
import uuid
import sys
from dotenv import load_dotenv

load_dotenv()

# Database connection parameters
# DB_CONFIG = {
#     'host': os.getenv('DB_HOST', 'localhost'),
#     'port': os.getenv('DB_PORT', '5433'),
#     'database': os.getenv('DB_NAME', 'llmlayer'),
#     'user': os.getenv('DB_USER', 'llmlayer'),
#     'password': os.getenv('DB_PASSWORD', 'llmlayer_dev')
# }

DB_CONFIG = {
    'host': os.getenv('DB_HOST'),
    'port': os.getenv('DB_PORT'),
    'database': os.getenv('DB_NAME'),
    'user': os.getenv('DB_USER'),
    'password': os.getenv('DB_PASSWORD')
}

CSV_FILE = "region_mapping.csv"

# SQL Schema for brain_region_responses table
CREATE_TABLE_SQL = """

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
DO $$ 
BEGIN
    CREATE DOMAIN brain_namespace AS UUID;
EXCEPTION WHEN OTHERS THEN
    NULL;
END $$;

CREATE TABLE brain_regions (
    id UUID PRIMARY KEY,  -- UUID v5 based on region name (deterministic)
    region_id INTEGER NOT NULL UNIQUE,  -- Original id from CSV
    name VARCHAR(255) NOT NULL UNIQUE,
    acronym VARCHAR(50),
    red INTEGER,
    green INTEGER,
    blue INTEGER,
    structure_order INTEGER,
    parent_region_id INTEGER REFERENCES brain_regions(region_id),  -- References CSV parent_id
    parent_acronym VARCHAR(50) REFERENCES brain_regions(acronym),  -- Reference to parent acronym
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Optional: Add an index on parent_region_id for faster hierarchical queries
CREATE INDEX idx_parent_region_id ON brain_regions(parent_region_id);

-- Optional: Add a check constraint to ensure RGB values are within valid range
ALTER TABLE brain_regions 
ADD CONSTRAINT check_rgb_values 
CHECK (red >= 0 AND red <= 255 AND green >= 0 AND green <= 255 AND blue >= 0 AND blue <= 255);

set BRAIN_NAMESPACE '550e8400-e29b-41d4-a716-446655440000'
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
        
        cursor.execute("DROP TABLE IF EXISTS brain_regions CASCADE;")
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


BRAIN_NAMESPACE = uuid.UUID('550e8400-e29b-41d4-a716-446655440000')

def generate_uuid_v5(region_name):
    """Generate a deterministic UUID v5 based on region name."""
    return uuid.uuid5(BRAIN_NAMESPACE, region_name)

def read_csv(filepath):
    """Read CSV file and return rows as list of dicts."""
    rows = []
    with open(filepath, 'r', encoding='utf-8') as f:
        reader = csv.DictReader(f)
        for row in reader:
            rows.append(row)
    return rows

def insert_data(csv_rows):
    """Insert CSV data into PostgreSQL database."""
    try:
        # Connect to database
        conn = get_connection()
        cursor = conn.cursor()

        # Prepare data for insertion
        insert_data = []
        for row in csv_rows:
            region_id = int(row['id'])
            name = row['name']
            acronym = row.get('acronym', None)
            red = int(row.get('red', 0)) if row.get('red') else None
            green = int(row.get('green', 0)) if row.get('green') else None
            blue = int(row.get('blue', 0)) if row.get('blue') else None
            structure_order = int(row.get('structure_order', 0)) if row.get('structure_order') else None
            parent_region_id = int(row.get('parent_id')) if row.get('parent_id') and row.get('parent_id').strip() else None
            parent_acronym = row.get('parent_acronym', None)

            # Generate UUID v5 from region name
            uuid_v5 = generate_uuid_v5(name)

            insert_data.append((
                uuid_v5,
                region_id,
                name,
                acronym,
                red,
                green,
                blue,
                structure_order,
                parent_region_id,
                parent_acronym
            ))

        # Bulk insert
        sql = """
            INSERT INTO brain_regions 
            (id, region_id, name, acronym, red, green, blue, structure_order, parent_region_id, parent_acronym)
            VALUES %s
            ON CONFLICT (region_id) DO NOTHING
        """

        execute_values(cursor, sql, insert_data)
        conn.commit()

        print(f"Successfully inserted {cursor.rowcount} rows into brain_regions table.")
        cursor.close()
        conn.close()

    except psycopg2.Error as e:
        print(f"Database error: {e}")
        sys.exit(1)
    except Exception as e:
        print(f"Error: {e}")
        sys.exit(1)


if __name__ == "__main__":
    print(f"Reading CSV file: {CSV_FILE}")
    csv_rows = read_csv(CSV_FILE)
    print(f"Found {len(csv_rows)} rows in CSV")
    print("Inserting data into PostgreSQL...")
    insert_data(csv_rows)
    print("Import complete!")

# PostgreSQL Database for LLM Responses

This directory contains the database schema and repository methods for storing LLM brain region responses in PostgreSQL.

## Database Configuration

The PostgreSQL database is configured in `docker-compose.yml`:
- **Host**: localhost
- **Port**: 5433 (mapped from container's 5432)
- **Database**: llmlayer
- **User**: llmlayer
- **Password**: llmlayer_dev

## Environment Variables

Add these to your `.env` file:

```bash
# Database configuration
DB_HOST=localhost
DB_PORT=5433
DB_NAME=llmlayer
DB_USER=llmlayer
DB_PASSWORD=llmlayer_dev
```

## Files

### `schema.py`
- Defines the PostgreSQL table schema for `brain_region_responses`
- Contains database initialization functions
- Run `python src/db/schema.py` to create the table

**Table Schema:**
```sql
CREATE TABLE brain_region_responses (
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
```

### `repository.py`
Contains methods for database operations:

**Storage Methods:**
- `store_brain_region_response()` - Store a BrainRegion LLM response

**Retrieval Methods:**
- `get_brain_region_response_by_id()` - Get response by ID
- `get_brain_region_responses_by_name()` - Get all responses for a region name
- `get_recent_responses()` - Get most recent responses
- `search_by_query()` - Search by query text
- `get_responses_by_hemisphere()` - Filter by hemisphere
- `get_statistics()` - Get database statistics

**Management Methods:**
- `delete_response()` - Delete a response by ID

### `example_usage.py`
Example script showing how to:
1. Initialize the database
2. Query the LLM about brain regions
3. Store responses in PostgreSQL
4. Retrieve and display stored data

## Quick Start

### 1. Start PostgreSQL
```bash
docker-compose up -d postgres
```

### 2. Initialize Database
```bash
python src/db/schema.py
```

### 3. Query and Store LLM Responses
```bash
python src/db/example_usage.py
```

Or use programmatically:
```python
from query.queryllm import brain_region_query
from db.repository import store_brain_region_response
from db.schema import init_database

# Initialize database (first time only)
init_database()

# Query LLM
brain_region = brain_region_query("Tell me about the hippocampus.", include_context=True)

# Store in database
record_id = store_brain_region_response(
    query="Tell me about the hippocampus.",
    brain_region=brain_region,
    model_name='deepseek-r1:8b',
    include_context=True
)

print(f"Stored with ID: {record_id}")
```

### 4. Query Stored Data
```python
from db.repository import (
    get_recent_responses,
    get_brain_region_responses_by_name,
    get_statistics
)

# Get recent responses
recent = get_recent_responses(limit=10)
for response in recent:
    print(f"{response['region_name']}: {response['query']}")

# Search by region name
hippocampus_responses = get_brain_region_responses_by_name("hippocampus")

# Get statistics
stats = get_statistics()
print(f"Total responses: {stats['total_responses']}")
print(f"Unique regions: {stats['unique_regions']}")
```

## Database Management

### View Database with pgAdmin
Access pgAdmin at http://localhost:5050
- **Email**: admin@cortexmap.com
- **Password**: admin

### View Database with Adminer
Access Adminer at http://localhost:8080
- **Server**: postgres:5432
- **Username**: llmlayer
- **Password**: llmlayer_dev
- **Database**: llmlayer

### Reset Database
```python
from db.schema import drop_table, init_database

# Drop and recreate table
drop_table()
init_database()
```

## Integration with Existing Code

The `queryllm.py` file already defines the `BrainRegion` Pydantic model. To integrate database storage:

```python
from query.queryllm import brain_region_query
from db.repository import store_brain_region_response

# Your existing query code
result = brain_region_query("Tell me about the prefrontal cortex.", include_context=True)

# Add database storage
record_id = store_brain_region_response(
    query="Tell me about the prefrontal cortex.",
    brain_region=result,
    model_name='deepseek-r1:8b',
    include_context=True
)
```

## Dependencies

All required dependencies are already in `requirements.txt`:
- `psycopg2-binary==2.9.11` - PostgreSQL adapter
- `python-dotenv==1.1.0` - Environment variable management
- `pydantic==2.11.9` - Data validation (already used)

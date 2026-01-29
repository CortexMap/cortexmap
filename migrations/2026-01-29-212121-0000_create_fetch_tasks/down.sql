-- Rollback fetch_tasks table creation
DROP TABLE IF EXISTS fetch_tasks CASCADE;

-- Drop the helper functions
DROP FUNCTION IF EXISTS diesel_manage_updated_at(regclass);
DROP FUNCTION IF EXISTS diesel_set_updated_at();

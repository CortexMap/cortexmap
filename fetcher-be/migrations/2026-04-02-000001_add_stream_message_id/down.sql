DROP INDEX IF EXISTS idx_fetch_tasks_stream_id;
ALTER TABLE fetch_tasks DROP COLUMN IF EXISTS stream_message_id;

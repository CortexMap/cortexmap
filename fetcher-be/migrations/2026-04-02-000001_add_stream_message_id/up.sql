ALTER TABLE fetch_tasks ADD COLUMN stream_message_id TEXT;
CREATE INDEX idx_fetch_tasks_stream_id ON fetch_tasks(stream_message_id)
  WHERE stream_message_id IS NOT NULL;

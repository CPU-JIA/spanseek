CREATE TABLE voice_sync_tasks (
  id TEXT PRIMARY KEY,
  capture_id TEXT,
  device_id TEXT,
  direction TEXT NOT NULL,
  status TEXT NOT NULL,
  blob_hash TEXT NOT NULL,
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  row_version INTEGER NOT NULL
);

CREATE UNIQUE INDEX voice_sync_tasks_blob_hash ON voice_sync_tasks(blob_hash);

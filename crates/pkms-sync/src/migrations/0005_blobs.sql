CREATE TABLE sync_blobs (
    blob_hash TEXT PRIMARY KEY,
    size INTEGER NOT NULL CHECK(size >= 1),
    chunk_size INTEGER NOT NULL CHECK(chunk_size >= 1),
    chunk_count INTEGER NOT NULL CHECK(chunk_count >= 1),
    status TEXT NOT NULL CHECK(status IN ('pending','complete')),
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL REFERENCES sync_devices(device_id),
    finalized_at TEXT,
    CHECK((status = 'complete') = (finalized_at IS NOT NULL))
) STRICT;
CREATE TABLE sync_blob_chunks (
    blob_hash TEXT NOT NULL REFERENCES sync_blobs(blob_hash) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL CHECK(chunk_index >= 0),
    chunk_len INTEGER NOT NULL CHECK(chunk_len >= 1),
    chunk_hash TEXT NOT NULL CHECK(length(chunk_hash) = 64),
    received_at TEXT NOT NULL,
    PRIMARY KEY(blob_hash, chunk_index)
) STRICT;
PRAGMA user_version = 5;
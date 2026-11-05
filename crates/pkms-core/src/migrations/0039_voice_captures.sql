-- Voice memos keep raw audio first; transcription is derived and retryable.
CREATE TABLE voice_captures (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','transcribing','ready','failed')),
    audio_blob_hash TEXT NOT NULL REFERENCES blobs(hash),
    audio_media_type TEXT NOT NULL,
    audio_bytes INTEGER NOT NULL CHECK(audio_bytes > 0),
    duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0),
    transcript TEXT NOT NULL DEFAULT '',
    transcript_source TEXT NOT NULL CHECK(transcript_source IN ('none','asr','manual')),
    source_kind TEXT NOT NULL CHECK(source_kind IN ('local','import')),
    device_label TEXT,
    language TEXT,
    error_code TEXT,
    note_id TEXT REFERENCES objects(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    row_version INTEGER NOT NULL CHECK(row_version > 0),
    trashed INTEGER NOT NULL DEFAULT 0 CHECK(trashed IN (0,1))
) STRICT;
CREATE INDEX voice_captures_order ON voice_captures(trashed, created_at DESC, id);
CREATE INDEX voice_captures_note ON voice_captures(note_id);
CREATE INDEX voice_captures_audio ON voice_captures(audio_blob_hash);

-- Workspace-scoped ASR endpoint. The API key lives only in the OS credential vault.
CREATE TABLE asr_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    endpoint TEXT NOT NULL,
    model TEXT NOT NULL,
    language TEXT NOT NULL,
    row_version INTEGER NOT NULL CHECK(row_version > 0),
    updated_at TEXT NOT NULL
);
INSERT INTO asr_settings(singleton,endpoint,model,language,row_version,updated_at)
VALUES(1,'','','',1,'1970-01-01T00:00:00Z');

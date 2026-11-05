-- Provider timestamps make transcript editing and subtitle export seekable.
ALTER TABLE voice_captures ADD COLUMN segments_json TEXT NOT NULL DEFAULT '[]'
    CHECK(json_valid(segments_json));

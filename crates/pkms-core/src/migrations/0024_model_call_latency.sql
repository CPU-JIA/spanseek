-- Wall-clock latency of one recorded model call and the time until the first
-- streamed token. Both stay NULL when the caller cannot measure them, so the
-- panel can report "unknown" instead of a zero. Rows still carry numbers only.
ALTER TABLE model_usage_events ADD COLUMN duration_ms INTEGER
    CHECK(duration_ms IS NULL OR duration_ms BETWEEN 0 AND 3600000);
ALTER TABLE model_usage_events ADD COLUMN first_token_ms INTEGER
    CHECK(first_token_ms IS NULL OR first_token_ms BETWEEN 0 AND 3600000);
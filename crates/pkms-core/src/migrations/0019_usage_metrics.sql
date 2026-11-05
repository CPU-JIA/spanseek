-- Local operational metrics for the statistics panel: retrieval latency and
-- model call/token usage. Rows never contain user text or prompts, are pruned
-- to a rolling window, and are restored with the rest of the workspace file.
CREATE TABLE retrieval_events(
    id INTEGER PRIMARY KEY,
    occurred_at TEXT NOT NULL,
    mode TEXT NOT NULL CHECK(mode IN ('lexical','semantic','hybrid')),
    duration_ms INTEGER NOT NULL CHECK(duration_ms BETWEEN 0 AND 3600000),
    result_count INTEGER NOT NULL CHECK(result_count BETWEEN 0 AND 100000)
) STRICT;
CREATE INDEX retrieval_events_time ON retrieval_events(occurred_at,id);
CREATE TABLE model_usage_events(
    id INTEGER PRIMARY KEY,
    occurred_at TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK(purpose IN ('answer','chat','embedding')),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER CHECK(input_tokens IS NULL OR input_tokens>=0),
    output_tokens INTEGER CHECK(output_tokens IS NULL OR output_tokens>=0),
    estimated INTEGER NOT NULL CHECK(estimated IN (0,1))
) STRICT;
CREATE INDEX model_usage_events_time ON model_usage_events(occurred_at,id);


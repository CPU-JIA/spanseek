CREATE TABLE citation_styles (
  id TEXT PRIMARY KEY CHECK(length(id) BETWEEN 1 AND 512),
  title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 300),
  kind TEXT NOT NULL CHECK(kind IN ('independent','dependent')),
  parent_id TEXT CHECK(parent_id IS NULL OR length(parent_id) BETWEEN 1 AND 512),
  xml TEXT NOT NULL CHECK(length(xml) BETWEEN 1 AND 2097152),
  created_at TEXT NOT NULL
) STRICT;
CREATE INDEX citation_styles_parent ON citation_styles(parent_id,id);
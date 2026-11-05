CREATE TABLE review_entries(
 id TEXT PRIMARY KEY,
 target_key TEXT NOT NULL UNIQUE,
 object_id TEXT NOT NULL REFERENCES objects(id),
 target_json TEXT NOT NULL CHECK(json_valid(target_json)),
 kind TEXT NOT NULL CHECK(kind IN ('note','source','annotation')),
 state TEXT NOT NULL CHECK(state IN ('pending','completed','removed')),
 due_at TEXT NOT NULL,
 row_version INTEGER NOT NULL CHECK(row_version>=1),
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX review_entries_due ON review_entries(state,due_at,id);
CREATE INDEX review_entries_updated ON review_entries(state,updated_at,id);
CREATE INDEX review_entries_object ON review_entries(object_id);
CREATE TABLE review_events(
 entry_id TEXT NOT NULL REFERENCES review_entries(id),
 seq INTEGER NOT NULL CHECK(seq>=1),
 action TEXT NOT NULL CHECK(action IN ('added','completed','snoozed','removed')),
 state TEXT NOT NULL CHECK(state IN ('pending','completed','removed')),
 due_at TEXT NOT NULL,
 occurred_at TEXT NOT NULL,
 PRIMARY KEY(entry_id,seq)
) STRICT;

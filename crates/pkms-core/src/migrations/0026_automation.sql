-- V1.9.5 controlled automation and explainable review.
--
-- Automation never writes knowledge by itself: a change set is an immutable
-- proposal, and each item is applied only through an explicit per-item command.
-- Every automation write records an audit event with the before/after payload
-- needed to reverse it. Research tasks record one bounded step at a time so a
-- cancel can be observed before the next step starts.

CREATE TABLE automation_changesets (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('tag_suggestions','duplicate_candidates','relation_suggestions','stale_sources')),
    status TEXT NOT NULL CHECK(status IN ('pending','applied','rejected','expired')),
    scope_json TEXT NOT NULL CHECK(json_valid(scope_json) AND json_type(scope_json)='object'),
    item_count INTEGER NOT NULL CHECK(item_count>=0),
    applied_count INTEGER NOT NULL DEFAULT 0 CHECK(applied_count>=0 AND applied_count<=item_count),
    row_version INTEGER NOT NULL CHECK(row_version>=1),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX automation_changesets_status ON automation_changesets(status,created_at,id);

CREATE TABLE automation_items (
    change_id TEXT NOT NULL REFERENCES automation_changesets(id),
    item_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    kind TEXT NOT NULL CHECK(kind IN ('assign_tag','trash_duplicate','append_relation','acknowledge_stale')),
    target_object_id TEXT NOT NULL,
    target_title TEXT NOT NULL,
    target_revision_id TEXT,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND json_type(payload_json)='object'),
    evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json) AND json_type(evidence_json)='array'),
    requires_confirmation INTEGER NOT NULL CHECK(requires_confirmation IN (0,1)),
    status TEXT NOT NULL CHECK(status IN ('pending','applied','skipped','failed')),
    blocked_reason TEXT,
    audit_id TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(change_id,item_id)
) STRICT;
CREATE INDEX automation_items_target ON automation_items(target_object_id,status);

CREATE TABLE audit_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    kind TEXT NOT NULL,
    subject_kind TEXT NOT NULL CHECK(subject_kind IN ('note','source','change_set','research_task','review_setting','tag')),
    subject_id TEXT NOT NULL,
    summary TEXT NOT NULL CHECK(length(summary) BETWEEN 1 AND 1000),
    origin TEXT NOT NULL CHECK(origin IN ('user','automation','system')),
    undoable INTEGER NOT NULL CHECK(undoable IN (0,1)),
    undo_json TEXT CHECK(undo_json IS NULL OR (json_valid(undo_json) AND json_type(undo_json)='object')),
    undone_by TEXT NOT NULL DEFAULT '',
    undone_at TEXT,
    change_id TEXT,
    item_id TEXT
) STRICT;
CREATE INDEX audit_events_time ON audit_events(occurred_at DESC,seq DESC);
CREATE INDEX audit_events_subject ON audit_events(subject_kind,subject_id,seq DESC);

CREATE TABLE research_tasks (
    id TEXT PRIMARY KEY,
    goal TEXT NOT NULL CHECK(length(goal) BETWEEN 1 AND 2000),
    collection_id TEXT,
    queries_json TEXT NOT NULL CHECK(json_valid(queries_json) AND json_type(queries_json)='array'),
    budget_json TEXT NOT NULL CHECK(json_valid(budget_json) AND json_type(budget_json)='object'),
    stop_json TEXT NOT NULL CHECK(json_valid(stop_json) AND json_type(stop_json)='object'),
    external_allowed INTEGER NOT NULL CHECK(external_allowed IN (0,1)),
    external_search_endpoint TEXT,
    status TEXT NOT NULL CHECK(status IN ('pending','running','completed','canceled','failed')),
    stop_reason TEXT,
    step_count INTEGER NOT NULL DEFAULT 0 CHECK(step_count>=0),
    row_version INTEGER NOT NULL CHECK(row_version>=1),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX research_tasks_status ON research_tasks(status,created_at,id);

CREATE TABLE research_steps (
    task_id TEXT NOT NULL REFERENCES research_tasks(id),
    seq INTEGER NOT NULL CHECK(seq>=1),
    kind TEXT NOT NULL CHECK(kind IN ('private_retrieval','external_search')),
    status TEXT NOT NULL CHECK(status IN ('succeeded','failed','canceled','skipped')),
    origin TEXT NOT NULL CHECK(origin IN ('private_library','external')),
    input_json TEXT NOT NULL CHECK(json_valid(input_json) AND json_type(input_json)='object'),
    result_json TEXT NOT NULL CHECK(json_valid(result_json) AND json_type(result_json)='object'),
    sources_json TEXT NOT NULL CHECK(json_valid(sources_json) AND json_type(sources_json)='array'),
    created_at TEXT NOT NULL,
    PRIMARY KEY(task_id,seq)
) STRICT;

CREATE TABLE research_evidence (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES research_tasks(id),
    step_seq INTEGER NOT NULL,
    object_id TEXT NOT NULL,
    object_revision_id TEXT,
    origin TEXT NOT NULL CHECK(origin IN ('private_library','external')),
    title TEXT NOT NULL,
    excerpt TEXT NOT NULL CHECK(length(excerpt)<=2000),
    excerpt_hash TEXT NOT NULL CHECK(length(excerpt_hash)=64),
    source_url TEXT,
    captured_at TEXT NOT NULL,
    UNIQUE(task_id,step_seq,object_id,excerpt_hash)
) STRICT;
CREATE INDEX research_evidence_task ON research_evidence(task_id,step_seq);

CREATE TABLE review_insight_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    window_days INTEGER NOT NULL CHECK(window_days BETWEEN 1 AND 365),
    include_completed INTEGER NOT NULL CHECK(include_completed IN (0,1)),
    include_stale INTEGER NOT NULL CHECK(include_stale IN (0,1)),
    row_version INTEGER NOT NULL CHECK(row_version>=1)
) STRICT;
-- Explainable review is opt-in: nothing is derived until the user enables it.
INSERT INTO review_insight_settings VALUES(1,0,30,1,1,1);
-- The current block projection extends the existing identity table. Individual
-- columns are added idempotently by the migration runner before this SQL runs.
CREATE INDEX IF NOT EXISTS blocks_note_active_position ON blocks(note_id,active,position);
CREATE INDEX IF NOT EXISTS blocks_parent_position ON blocks(parent_block_id,position);
CREATE UNIQUE INDEX IF NOT EXISTS blocks_active_alias ON blocks(note_id,block_alias) WHERE active=1 AND block_alias IS NOT NULL;
CREATE INDEX IF NOT EXISTS blocks_active_revision ON blocks(revision_id) WHERE active=1;

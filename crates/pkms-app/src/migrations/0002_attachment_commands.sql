-- Bind native file grants before entering the knowledge database. A crash after
-- core commit can then replay that receipt without touching the original file.
CREATE TABLE attachment_import_commands (
    local_instance_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    token TEXT NOT NULL,
    captured_hash TEXT NOT NULL,
    original_name TEXT NOT NULL,
    PRIMARY KEY (local_instance_id, workspace_id, command_id)
) STRICT;
PRAGMA user_version = 2;

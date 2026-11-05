ALTER TABLE app_operations ADD COLUMN output_token TEXT REFERENCES local_resource_grants(token);
ALTER TABLE app_operations ADD COLUMN report_json TEXT CHECK(report_json IS NULL OR json_valid(report_json));
CREATE TABLE workspace_job_outputs (
    local_instance_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    token TEXT NOT NULL REFERENCES local_resource_grants(token),
    input_json TEXT NOT NULL CHECK(json_valid(input_json)),
    job_id TEXT,
    checkpoint_json TEXT CHECK(checkpoint_json IS NULL OR json_valid(checkpoint_json)),
    PRIMARY KEY(local_instance_id,workspace_id,command_id),
    UNIQUE(local_instance_id,workspace_id,job_id),
    FOREIGN KEY(local_instance_id,workspace_id) REFERENCES workspace_catalog(local_instance_id,workspace_id)
) STRICT;
PRAGMA user_version = 3;

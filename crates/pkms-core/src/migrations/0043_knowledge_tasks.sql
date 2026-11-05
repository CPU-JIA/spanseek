CREATE TABLE knowledge_task_templates (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    instructions TEXT NOT NULL,
    row_version INTEGER NOT NULL CHECK(row_version>0),
    is_example INTEGER NOT NULL CHECK(is_example IN (0,1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

-- These editable examples are data, never execution types. Deletion is persistent.
INSERT INTO knowledge_task_templates VALUES
('6c6d70a1-f2d9-4d92-a10a-2236a2c20301','研究汇报','研究汇报','整理问题、发现、局限和下一步。','根据所选材料组织研究问题、材料支持的发现、局限与待核实点、汇报提纲和后续方向。',1,1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('6c6d70a1-f2d9-4d92-a10a-2236a2c20302','灵感整理','灵感整理','找出线索、反例与值得尝试的想法。','梳理所选材料中的想法，归纳共同线索、反例与待验证问题，列出可尝试的小实验。',1,1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('6c6d70a1-f2d9-4d92-a10a-2236a2c20303','工作复盘','工作复盘','从记录中提炼经验和具体行动。','依据材料对照原定目标与已记录的结果，提炼决策、经验、未解决问题和下一步。不要编造绩效数字。',1,1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('6c6d70a1-f2d9-4d92-a10a-2236a2c20304','辅助写作','辅助写作','将材料组织为可继续编辑的草稿。','围绕本次目标把所选材料组织为写作草稿，保留材料原意，区分有材料支持的表述与写作建议，并给出简短的修改检查项。',1,1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'));

CREATE TABLE knowledge_tasks (
    id TEXT PRIMARY KEY REFERENCES chat_runs(id),
    template_id TEXT REFERENCES knowledge_task_templates(id) ON DELETE SET NULL,
    template_snapshot_json TEXT CHECK(template_snapshot_json IS NULL OR json_valid(template_snapshot_json)),
    title TEXT NOT NULL,
    input_schema INTEGER NOT NULL CHECK(input_schema=1),
    input_json TEXT NOT NULL CHECK(json_valid(input_json)),
    channel_name TEXT,
    retry_of TEXT REFERENCES knowledge_tasks(id),
    created_at TEXT NOT NULL
) STRICT;
CREATE INDEX knowledge_tasks_history ON knowledge_tasks(created_at DESC,id DESC);
CREATE INDEX knowledge_task_proposal_origin ON proposals(json_extract(request_json,'$.chat_run_id'));
CREATE TABLE knowledge_task_outputs (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES knowledge_tasks(id),
    proposal_id TEXT UNIQUE REFERENCES proposals(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','applied','rejected')),
    target_note_id TEXT,
    applied_note_id TEXT,
    applied_revision_id TEXT,
    created_at TEXT NOT NULL
) STRICT;
CREATE INDEX knowledge_task_outputs_history ON knowledge_task_outputs(task_id,created_at DESC,id DESC);
CREATE TRIGGER knowledge_task_proposal_created AFTER INSERT ON proposals
WHEN EXISTS(SELECT 1 FROM knowledge_tasks t WHERE t.id=json_extract(NEW.request_json,'$.chat_run_id'))
BEGIN
    INSERT INTO knowledge_task_outputs(id,task_id,proposal_id,title,status,target_note_id,applied_note_id,applied_revision_id,created_at)
    VALUES(NEW.id,json_extract(NEW.request_json,'$.chat_run_id'),NEW.id,json_extract(NEW.request_json,'$.title'),NEW.status,json_extract(NEW.request_json,'$.note_id'),
        (SELECT note_id FROM note_revisions WHERE revision_id=NEW.applied_revision_id),NEW.applied_revision_id,NEW.created_at);
END;
CREATE TRIGGER knowledge_task_proposal_updated AFTER UPDATE OF status,applied_revision_id ON proposals
BEGIN
    UPDATE knowledge_task_outputs SET status=NEW.status,
        applied_note_id=(SELECT note_id FROM note_revisions WHERE revision_id=NEW.applied_revision_id),
        applied_revision_id=NEW.applied_revision_id WHERE proposal_id=NEW.id;
END;

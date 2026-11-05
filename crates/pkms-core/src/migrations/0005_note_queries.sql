CREATE INDEX notes_page_order ON notes(trashed, updated_at DESC, id ASC);
CREATE TABLE note_list_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    generation INTEGER NOT NULL CHECK(generation>0)
) STRICT;
INSERT INTO note_list_state VALUES(1,1);
CREATE TABLE note_previews (
    note_id TEXT PRIMARY KEY REFERENCES notes(id),
    revision_id TEXT NOT NULL REFERENCES note_revisions(revision_id),
    preview TEXT NOT NULL CHECK(length(preview)<=180)
) STRICT;
-- A disposable display excerpt; full documents remain authoritative and are
-- validated on read. Parse each changed document once, never on every list page.
INSERT INTO note_previews
SELECT n.id,r.revision_id,COALESCE((SELECT substr(group_concat(value,' '),1,180)
    FROM json_tree(r.document_json) WHERE key='text' AND type='text'),'')
FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id;
CREATE TRIGGER note_preview_changed AFTER UPDATE OF current_revision_id ON notes
WHEN NEW.current_revision_id IS NOT NULL BEGIN
    INSERT INTO note_previews
    SELECT NEW.id,r.revision_id,COALESCE((SELECT substr(group_concat(value,' '),1,180)
        FROM json_tree(r.document_json) WHERE key='text' AND type='text'),'')
    FROM note_revisions r WHERE r.revision_id=NEW.current_revision_id
    ON CONFLICT(note_id) DO UPDATE SET revision_id=excluded.revision_id,preview=excluded.preview;
END;
CREATE TRIGGER note_list_insert AFTER INSERT ON notes BEGIN
    UPDATE note_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER note_list_update AFTER UPDATE ON notes BEGIN
    UPDATE note_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER note_list_delete AFTER DELETE ON notes BEGIN
    UPDATE note_list_state SET generation=generation+1 WHERE singleton=1;
END;

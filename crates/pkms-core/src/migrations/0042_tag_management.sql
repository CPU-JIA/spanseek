-- Keep existing tag identities and names, including receipt snapshots.
ALTER TABLE tags ADD COLUMN description TEXT NOT NULL DEFAULT '' CHECK(length(description)<=4000);
ALTER TABLE tags ADD COLUMN row_version INTEGER NOT NULL DEFAULT 1 CHECK(row_version BETWEEN 1 AND 4294967295);
ALTER TABLE tags ADD COLUMN created_at TEXT;
ALTER TABLE tags ADD COLUMN updated_at TEXT;

-- Existing assignment/import/automation paths create tags too. Timestamp only
-- future inserts; old tags retain NULL instead of a fabricated history date.
CREATE TRIGGER tag_details_insert AFTER INSERT ON tags BEGIN
    UPDATE tags SET
        created_at=COALESCE(NEW.created_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        updated_at=COALESCE(NEW.updated_at,strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    WHERE id=NEW.id;
END;
CREATE TRIGGER tag_name_invalidate_notes AFTER UPDATE OF display_name,normalized_name ON tags
WHEN OLD.display_name<>NEW.display_name OR OLD.normalized_name<>NEW.normalized_name BEGIN
    UPDATE note_list_state SET generation=generation+1 WHERE singleton=1;
END;

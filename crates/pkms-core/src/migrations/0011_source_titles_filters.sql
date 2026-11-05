CREATE VIRTUAL TABLE source_title_fts USING fts5(source_id UNINDEXED,title_tokens);

CREATE TRIGGER source_reading_insert AFTER INSERT ON reading_states BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_reading_update AFTER UPDATE ON reading_states BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_reading_delete AFTER DELETE ON reading_states BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_tag_insert AFTER INSERT ON object_tags WHEN EXISTS(SELECT 1 FROM objects WHERE id=NEW.object_id AND kind='source') BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_tag_update AFTER UPDATE ON object_tags WHEN EXISTS(SELECT 1 FROM objects WHERE id IN (NEW.object_id,OLD.object_id) AND kind='source') BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_tag_delete AFTER DELETE ON object_tags WHEN EXISTS(SELECT 1 FROM objects WHERE id=OLD.object_id AND kind='source') BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_membership_insert AFTER INSERT ON memberships WHEN EXISTS(SELECT 1 FROM objects WHERE id=NEW.member_id AND kind='source') BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_membership_update AFTER UPDATE ON memberships WHEN EXISTS(SELECT 1 FROM objects WHERE id IN (NEW.member_id,OLD.member_id) AND kind='source') BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_membership_delete AFTER DELETE ON memberships WHEN EXISTS(SELECT 1 FROM objects WHERE id=OLD.member_id AND kind='source') BEGIN UPDATE source_list_state SET generation=generation+1; END;

//! Shared fixtures for the frozen-schema package tests. Each integration test
//! compiles this module separately, so unused helpers are expected.
#![allow(dead_code)]

/// Remove K39-K44 additions before rebuilding a fixture for an older schema.
/// Triggers must go first: they reference the tables and tag columns removed
/// below, including from writes made by the remaining fixture SQL.
fn pre_voice_capture_versions() -> &'static str {
    "DROP TRIGGER tag_sync_dictionary_insert; \
     DROP TRIGGER tag_sync_dictionary_update; \
     DROP TRIGGER tag_sync_dictionary_delete; \
     DROP TRIGGER tag_sync_assignment_insert; \
     DROP TRIGGER tag_sync_assignment_delete; \
     DROP TABLE tag_sync_state; \
     DROP TABLE tag_sync_tombstones; \
     DROP TRIGGER knowledge_task_proposal_created; \
     DROP TRIGGER knowledge_task_proposal_updated; \
     DROP INDEX knowledge_task_proposal_origin; \
     DROP TABLE knowledge_task_outputs; \
     DROP TABLE knowledge_tasks; \
     DROP TABLE knowledge_task_templates; \
     DROP TRIGGER tag_details_insert; \
     DROP TRIGGER tag_name_invalidate_notes; \
     ALTER TABLE tags DROP COLUMN description; \
     ALTER TABLE tags DROP COLUMN row_version; \
     ALTER TABLE tags DROP COLUMN created_at; \
     ALTER TABLE tags DROP COLUMN updated_at; \
     DROP TABLE voice_sync_tasks; \
     DROP TABLE voice_captures; \
     DROP TABLE asr_settings; \
     DELETE FROM migration_history WHERE version>=39;"
}

/// Rewind the history used by block backfill tests. The K33 installer accepts
/// existing projection columns, so keep their identities and clear derived data.
pub fn block_backfill_schema32() -> String {
    format!(
        "{later} \
         UPDATE blocks SET revision_id=NULL,parent_block_id=NULL,position=NULL,depth=NULL,kind=NULL,text=NULL,block_alias=NULL,updated_at=NULL; \
         DROP TABLE source_block_images; DROP TABLE source_assets; DROP TABLE source_covers; \
         ALTER TABLE source_parses DROP COLUMN toc_json; \
         DROP TABLE workspace_metadata; DROP INDEX notes_type_key_order; ALTER TABLE notes DROP COLUMN type_key; \
         DELETE FROM migration_history WHERE version>=33; \
         UPDATE workspace_meta SET schema_version=32; PRAGMA user_version=32;",
        later = pre_voice_capture_versions()
    )
}

/// Pre-20 packages predate the DOCX format gate, and schema 20 rebuilds
/// `source_versions` through a rename that rewrites its stored DDL text. A
/// fixture for an older target must therefore rebuild the table from the frozen
/// v9 DDL, so the exact DDL validator still checks a real old schema instead of
/// today's table.
pub fn pre_docx_source_versions() -> String {
    let table = include_str!("../../src/migrations/0009_sources.sql")
        .split_once("CREATE TABLE source_versions")
        .unwrap()
        .1
        .split_once(';')
        .unwrap()
        .0;
    let blocks = include_str!("../../src/migrations/0003_rich_notes.sql")
        .split_once("CREATE TABLE blocks")
        .unwrap()
        .1
        .split_once(';')
        .unwrap()
        .0;
    let source = include_str!("../../src/migrations/0012_retrieval_assistant.sql");
    let trigger = |name: &str| -> String {
        source
            .split_once(&format!("CREATE TRIGGER {name}"))
            .unwrap()
            .1
            .split_once("END;")
            .unwrap()
            .0
            .to_owned()
    };
    let insert = trigger("index_dirty_parse_insert");
    let update = trigger("index_dirty_parse_update");
    format!(
        "{later} \
         DROP TABLE IF EXISTS workspace_metadata; \
         DROP INDEX IF EXISTS notes_type_key_order; \
         ALTER TABLE notes DROP COLUMN type_key; \
         DROP TABLE IF EXISTS source_block_images; \
         DROP TABLE IF EXISTS source_assets; \
         DROP TABLE IF EXISTS source_covers; \
         ALTER TABLE source_parses DROP COLUMN toc_json; \
         DROP TABLE IF EXISTS citation_styles; \
         DELETE FROM migration_history WHERE version>=28; \
         PRAGMA foreign_keys=OFF; \
         CREATE TEMP TABLE pkms_fixture_blocks AS SELECT id,note_id,active FROM blocks; \
         DROP TABLE blocks; CREATE TABLE blocks{blocks}; \
         INSERT INTO blocks SELECT id,note_id,active FROM pkms_fixture_blocks; \
         DROP TABLE pkms_fixture_blocks; \
         CREATE TEMP TABLE pkms_fixture_versions AS SELECT * FROM source_versions; \
         DROP TRIGGER index_dirty_parse_insert; DROP TRIGGER index_dirty_parse_update; \
         DROP TABLE source_versions; CREATE TABLE source_versions{table}; \
         INSERT INTO source_versions SELECT * FROM pkms_fixture_versions; \
         DROP TABLE pkms_fixture_versions; \
         CREATE INDEX source_versions_blob ON source_versions(blob_hash,source_id); \
         CREATE INDEX source_versions_history ON source_versions(source_id,captured_at DESC,id DESC); \
         CREATE TRIGGER index_dirty_parse_insert{insert}END; \
         CREATE TRIGGER index_dirty_parse_update{update}END;",
        later = pre_voice_capture_versions()
    )
}

/// Fixtures that pretend to predate K25 must also remove the K25 tables and
/// history row. Leaving them behind makes the migration-history count disagree
/// with the declared schema version and makes 0025 collide with existing
/// tables when the fixture is upgraded again.
pub fn pre_views_canvas_versions() -> String {
    let views = include_str!("../../src/migrations/0007_collections_views.sql");
    let table = views
        .split_once("CREATE TABLE saved_views")
        .unwrap()
        .1
        .split_once(';')
        .unwrap()
        .0;
    let trigger = |name: &str| -> String {
        views
            .split_once(&format!("CREATE TRIGGER {name}"))
            .unwrap()
            .1
            .split_once("END;")
            .unwrap()
            .0
            .to_owned()
    };
    let insert = trigger("view_list_insert");
    let update = trigger("view_list_update");
    let delete = trigger("view_list_delete");
    format!(
        "PRAGMA foreign_keys=OFF; \
         DROP TRIGGER IF EXISTS view_list_insert; \
         DROP TRIGGER IF EXISTS view_list_update; \
         DROP TRIGGER IF EXISTS view_list_delete; \
         CREATE TEMP TABLE pkms_fixture_views AS SELECT id,name,filter_schema,filter_ast,sort,\
         (CASE WHEN layout IN ('list','table') THEN layout ELSE 'list' END) AS layout,\
         is_pinned,pin_position,row_version,created_at,updated_at FROM saved_views; \
         DROP TABLE saved_views; \
         CREATE TABLE saved_views{table}; \
         INSERT INTO saved_views(id,name,filter_schema,filter_ast,sort,layout,is_pinned,pin_position,row_version,created_at,updated_at) \
         SELECT id,name,filter_schema,filter_ast,sort,layout,is_pinned,pin_position,row_version,created_at,updated_at FROM pkms_fixture_views; \
         DROP TABLE pkms_fixture_views; \
         DROP TABLE IF EXISTS canvases; \
         DROP TABLE IF EXISTS canvas_list_state; \
         DROP TABLE IF EXISTS sync_config_tombstones; \
         DELETE FROM migration_history WHERE version=25; \
         CREATE TRIGGER view_list_insert{insert}END; \
         CREATE TRIGGER view_list_update{update}END; \
         CREATE TRIGGER view_list_delete{delete}END; \
         PRAGMA foreign_keys=ON;",
        table = table,
        insert = insert,
        update = update,
        delete = delete
    )
}
/// Fixtures that declare a schema below 26 must remove every later table and
/// history row. Otherwise the exact history count disagrees with the declared
/// version, or a later migration collides with a table left behind.
pub fn pre_automation_versions() -> &'static str {
    "PRAGMA foreign_keys=OFF; \
     DROP TABLE IF EXISTS automation_items; \
     DROP TABLE IF EXISTS automation_changesets; \
     DROP TABLE IF EXISTS audit_events; \
     DROP TABLE IF EXISTS research_evidence; \
     DROP TABLE IF EXISTS research_steps; \
     DROP TABLE IF EXISTS research_tasks; \
     DROP TABLE IF EXISTS review_insight_settings; \
     DROP TABLE IF EXISTS external_audit_events; \
     DELETE FROM migration_history WHERE version IN (26,27); \
     PRAGMA foreign_keys=ON;"
}

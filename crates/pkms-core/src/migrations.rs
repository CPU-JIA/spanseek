//! Versioned, transactional upgrades. Existing user content is never reserialized.
use crate::{Error, Result, snapshot};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

const APPLICATION_ID: i64 = 1_347_112_275;
/// Published so tests and callers compare against the real current schema instead of a literal.
pub const CURRENT_SCHEMA_VERSION: i64 = 44;
const BASELINE: &str = include_str!("schema.sql");
const WORKSPACE_MIGRATION: &str = include_str!("migrations/0002_workspace.sql");
const RICH_NOTES_MIGRATION: &str = include_str!("migrations/0003_rich_notes.sql");
const ATTACHMENTS_MIGRATION: &str = include_str!("migrations/0004_attachments.sql");
const NOTE_QUERIES_MIGRATION: &str = include_str!("migrations/0005_note_queries.sql");
const ORGANIZATION_MIGRATION: &str = include_str!("migrations/0006_organization.sql");
const COLLECTIONS_VIEWS_MIGRATION: &str = include_str!("migrations/0007_collections_views.sql");
const PORTABLE_JOBS_MIGRATION: &str = include_str!("migrations/0008_portable_jobs.sql");
const SOURCES_MIGRATION: &str = include_str!("migrations/0009_sources.sql");
const WEB_CAPTURES_MIGRATION: &str = include_str!("migrations/0010_web_captures.sql");
const SOURCE_TITLES_MIGRATION: &str = include_str!("migrations/0011_source_titles_filters.sql");
const RETRIEVAL_MIGRATION: &str = include_str!("migrations/0012_retrieval_assistant.sql");
const REVIEW_MIGRATION: &str = include_str!("migrations/0013_review.sql");
const CHAT_MIGRATION: &str = include_str!("migrations/0014_chat.sql");
const RICH_FORMAT_MIGRATION: &str = include_str!("migrations/0015_rich_format.sql");
const GENERATION_CHANNELS_MIGRATION: &str = include_str!("migrations/0016_generation_channels.sql");
const READING_VIEW_MIGRATION: &str = include_str!("migrations/0017_reading_view.sql");
const RETRIEVAL_PARAMS_MIGRATION: &str = include_str!("migrations/0018_retrieval_params.sql");
const USAGE_METRICS_MIGRATION: &str = include_str!("migrations/0019_usage_metrics.sql");
const DOCX_FORMAT_MIGRATION: &str = include_str!("migrations/0020_docx_format.sql");
const EPUB_FORMAT_MIGRATION: &str = include_str!("migrations/0028_epub_format.sql");
const SPREADSHEET_FORMATS_MIGRATION: &str = include_str!("migrations/0029_spreadsheet_formats.sql");
const PPTX_FORMAT_MIGRATION: &str = include_str!("migrations/0030_pptx_format.sql");
const MOBI_FORMAT_MIGRATION: &str = include_str!("migrations/0031_mobi_format.sql");
const CITATION_STYLES_MIGRATION: &str = include_str!("migrations/0032_citation_styles.sql");
const BLOCK_DATABASE_MIGRATION: &str = include_str!("migrations/0033_block_database.sql");
const SOURCE_BOOK_NAVIGATION_MIGRATION: &str =
    include_str!("migrations/0034_source_book_navigation.sql");
const SOURCE_IMAGES_MIGRATION: &str = include_str!("migrations/0035_source_images.sql");
const XLS_FORMAT_MIGRATION: &str = include_str!("migrations/0036_xls_format.sql");
const DOC_FORMAT_MIGRATION: &str = include_str!("migrations/0037_doc_format.sql");
const WORKSPACE_METADATA_MIGRATION: &str = include_str!("migrations/0038_workspace_metadata.sql");
const VOICE_CAPTURES_MIGRATION: &str = include_str!("migrations/0039_voice_captures.sql");
const VOICE_SEGMENTS_MIGRATION: &str = include_str!("migrations/0040_voice_segments.sql");
const VOICE_SYNC_TASKS_MIGRATION: &str = include_str!("migrations/0041_voice_sync_tasks.sql");
const TAG_MANAGEMENT_MIGRATION: &str = include_str!("migrations/0042_tag_management.sql");
const KNOWLEDGE_TASKS_MIGRATION: &str = include_str!("migrations/0043_knowledge_tasks.sql");
const TAG_SYNC_MIGRATION: &str = include_str!("migrations/0044_tag_sync.sql");
const SOURCE_AUTHOR_MIGRATION: &str = include_str!("migrations/0021_source_author.sql");
const TEXT_CLEANING_MIGRATION: &str = include_str!("migrations/0022_text_cleaning.sql");
const CHUNK_SETTINGS_MIGRATION: &str = include_str!("migrations/0023_chunk_settings.sql");
const MODEL_CALL_LATENCY_MIGRATION: &str = include_str!("migrations/0024_model_call_latency.sql");
const VIEWS_CANVAS_MIGRATION: &str = include_str!("migrations/0025_views_canvas.sql");
const AUTOMATION_MIGRATION: &str = include_str!("migrations/0026_automation.sql");
const EXTERNAL_AUDIT_MIGRATION: &str = include_str!("migrations/0027_external_audit.sql");
const MIGRATIONS: [(i64, &str); 44] = [
    (1, BASELINE),
    (2, WORKSPACE_MIGRATION),
    (3, RICH_NOTES_MIGRATION),
    (4, ATTACHMENTS_MIGRATION),
    (5, NOTE_QUERIES_MIGRATION),
    (6, ORGANIZATION_MIGRATION),
    (7, COLLECTIONS_VIEWS_MIGRATION),
    (8, PORTABLE_JOBS_MIGRATION),
    (9, SOURCES_MIGRATION),
    (10, WEB_CAPTURES_MIGRATION),
    (11, SOURCE_TITLES_MIGRATION),
    (12, RETRIEVAL_MIGRATION),
    (13, REVIEW_MIGRATION),
    (14, CHAT_MIGRATION),
    (15, RICH_FORMAT_MIGRATION),
    (16, GENERATION_CHANNELS_MIGRATION),
    (17, READING_VIEW_MIGRATION),
    (18, RETRIEVAL_PARAMS_MIGRATION),
    (19, USAGE_METRICS_MIGRATION),
    (20, DOCX_FORMAT_MIGRATION),
    (21, SOURCE_AUTHOR_MIGRATION),
    (22, TEXT_CLEANING_MIGRATION),
    (23, CHUNK_SETTINGS_MIGRATION),
    (24, MODEL_CALL_LATENCY_MIGRATION),
    (25, VIEWS_CANVAS_MIGRATION),
    (26, AUTOMATION_MIGRATION),
    (27, EXTERNAL_AUDIT_MIGRATION),
    (28, EPUB_FORMAT_MIGRATION),
    (29, SPREADSHEET_FORMATS_MIGRATION),
    (30, PPTX_FORMAT_MIGRATION),
    (31, MOBI_FORMAT_MIGRATION),
    (32, CITATION_STYLES_MIGRATION),
    (33, BLOCK_DATABASE_MIGRATION),
    (34, SOURCE_BOOK_NAVIGATION_MIGRATION),
    (35, SOURCE_IMAGES_MIGRATION),
    (36, XLS_FORMAT_MIGRATION),
    (37, DOC_FORMAT_MIGRATION),
    (38, WORKSPACE_METADATA_MIGRATION),
    (39, VOICE_CAPTURES_MIGRATION),
    (40, VOICE_SEGMENTS_MIGRATION),
    (41, VOICE_SYNC_TASKS_MIGRATION),
    (42, TAG_MANAGEMENT_MIGRATION),
    (43, KNOWLEDGE_TASKS_MIGRATION),
    (44, TAG_SYNC_MIGRATION),
];

#[cfg(test)]
#[path = "rich_format_migration_tests.rs"]
mod rich_format_tests;

#[cfg(test)]
#[path = "migration_chain_tests.rs"]
mod migration_chain_tests;

pub(crate) struct OpenedDatabase {
    // Field order matters: close SQLite before releasing the workspace lease.
    pub connection: Connection,
    pub lock: File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DatabaseState {
    Empty,
    Version(i64),
}

pub(crate) fn open_database(path: &Path, create: bool) -> Result<OpenedDatabase> {
    // Do not create even a lock file for a mistyped existing-workspace path.
    if !create && !path.is_file() {
        return Err(
            io::Error::new(io::ErrorKind::NotFound, "workspace database is missing").into(),
        );
    }
    let path = canonical_database_path(path)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path(&path))?;
    shared_lock(&lock)?;
    let mut connection = connect(&path, create)?;
    let state = database_state(&connection, create)?;
    if state == DatabaseState::Version(CURRENT_SCHEMA_VERSION) {
        validate_history(&connection)?;
    } else {
        // There is no portable atomic shared-to-exclusive promotion. Close SQLite,
        // release our shared lease, then try exclusive and inspect again. Another
        // opener can have completed the upgrade in the meantime.
        drop(connection);
        lock.unlock()?;
        match lock.try_lock() {
            Ok(()) => {
                connection = connect(&path, create)?;
                let state = database_state(&connection, create)?;
                match state {
                    DatabaseState::Empty => initialize(&mut connection)?,
                    DatabaseState::Version(CURRENT_SCHEMA_VERSION) => {
                        validate_history(&connection)?
                    }
                    DatabaseState::Version(version) if version < CURRENT_SCHEMA_VERSION => {
                        upgrade(&mut connection, &path)?
                    }
                    DatabaseState::Version(_) => return Err(Error::UnsupportedDatabase),
                }
                lock.unlock()?;
                shared_lock(&lock)?;
            }
            Err(TryLockError::WouldBlock) => {
                // The winner may have finished migration and now hold a normal
                // shared lease. Rejoin readers instead of reporting a false busy.
                shared_lock(&lock)?;
                connection = connect(&path, create)?;
                if database_state(&connection, create)?
                    != DatabaseState::Version(CURRENT_SCHEMA_VERSION)
                {
                    return Err(Error::WorkspaceBusy);
                }
            }
            Err(error) => return Err(lock_error(error)),
        }
        // A newer executable might have upgraded in the brief unlocked interval.
        if database_state(&connection, false)? != DatabaseState::Version(CURRENT_SCHEMA_VERSION) {
            return Err(Error::UnsupportedDatabase);
        }
        validate_history(&connection)?;
    }
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(OpenedDatabase { connection, lock })
}

fn canonical_database_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or(Error::InvalidInput("database path must name a file"))?;
    Ok(parent.canonicalize()?.join(name))
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".pkms-lock");
    PathBuf::from(name)
}

fn lock_error(error: TryLockError) -> Error {
    match error {
        TryLockError::WouldBlock => Error::WorkspaceBusy,
        TryLockError::Error(error) => Error::Io(error),
    }
}

fn shared_lock(lock: &File) -> Result<()> {
    lock.try_lock_shared().map_err(lock_error)
}

fn connect(path: &Path, create: bool) -> Result<Connection> {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE;
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let connection = Connection::open_with_flags(path, flags)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(connection)
}

fn database_state(connection: &Connection, create: bool) -> Result<DatabaseState> {
    let app: i64 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if app == 0 && version == 0 && create {
        let objects: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        if objects == 0 {
            return Ok(DatabaseState::Empty);
        }
    }
    if app != APPLICATION_ID || !(1..=CURRENT_SCHEMA_VERSION).contains(&version) {
        return Err(Error::UnsupportedDatabase);
    }
    Ok(DatabaseState::Version(version))
}

fn checksum(sql: &str) -> String {
    // Git can check out CRLF on Windows. The published SQL is hashed as UTF-8 LF.
    format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()))
}

pub(crate) fn initialize(connection: &mut Connection) -> Result<()> {
    initialize_at(connection, CURRENT_SCHEMA_VERSION)
}

pub(crate) fn initialize_at(connection: &mut Connection, version: i64) -> Result<()> {
    if !(7..=CURRENT_SCHEMA_VERSION).contains(&version) {
        return Err(Error::UnsupportedDatabase);
    }
    if version >= 15 {
        return with_foreign_keys_restored(connection, |connection| {
            initialize_transaction(connection, version)
        });
    }
    initialize_transaction(connection, version)
}

fn initialize_transaction(connection: &mut Connection, version: i64) -> Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if database_state(&tx, true)? != DatabaseState::Empty {
        return Err(Error::UnsupportedDatabase);
    }
    tx.execute_batch(BASELINE)?;
    install_workspace_metadata(&tx)?;
    install_rich_notes(&tx)?;
    install_attachments(&tx)?;
    install_note_queries(&tx)?;
    install_organization(&tx)?;
    install_collections_views(&tx)?;
    if version >= 8 {
        install_portable_jobs(&tx)?;
    }
    if version >= 9 {
        install_sources(&tx)?;
    }
    if version >= 10 {
        install_web_captures(&tx)?;
    }
    if version >= 11 {
        install_source_titles(&tx)?;
    }
    if version >= 12 {
        install_retrieval(&tx)?;
    }
    if version >= 13 {
        install_review(&tx)?;
    }
    if version >= 14 {
        install_chat(&tx)?;
    }
    if version >= 15 {
        install_rich_format(&tx)?;
    }
    if version >= 16 {
        install_generation_channels(&tx)?;
    }
    if version >= 17 {
        install_reading_view(&tx)?;
    }
    if version >= 18 {
        install_retrieval_params(&tx)?;
    }
    if version >= 19 {
        install_usage_metrics(&tx)?;
    }
    if version >= 20 {
        install_docx_format(&tx)?;
    }
    if version >= 21 {
        install_source_author(&tx)?;
    }
    if version >= 22 {
        install_text_cleaning(&tx)?;
    }
    if version >= 23 {
        install_chunk_settings(&tx)?;
    }
    if version >= 24 {
        install_model_call_latency(&tx)?;
    }
    if version >= 25 {
        install_views_canvas(&tx)?;
    }
    if version >= 26 {
        install_automation(&tx)?;
    }
    if version >= 27 {
        install_external_audit(&tx)?;
    }
    if version >= 28 {
        install_epub_format(&tx)?;
    }
    if version >= 29 {
        install_spreadsheet_formats(&tx)?;
    }
    if version >= 30 {
        install_pptx_format(&tx)?;
    }
    if version >= 31 {
        install_mobi_format(&tx)?;
    }
    if version >= 32 {
        install_citation_styles(&tx)?;
    }
    if version >= 33 {
        install_block_database(&tx)?;
    }
    if version >= 34 {
        install_source_book_navigation(&tx)?;
    }
    if version >= 35 {
        install_source_images(&tx)?;
    }
    if version >= 36 {
        install_xls_format(&tx)?;
    }
    if version >= 37 {
        install_doc_format(&tx)?;
    }
    if version >= 38 {
        install_workspace_metadata_v38(&tx)?;
    }
    if version >= 39 {
        install_voice_captures(&tx)?;
    }
    if version >= 40 {
        install_voice_segments(&tx)?;
    }
    if version >= 41 {
        install_voice_sync_tasks(&tx)?;
    }
    if version >= 42 {
        install_tag_management(&tx)?;
    }
    if version >= 43 {
        install_knowledge_tasks(&tx)?;
    }
    if version >= 44 {
        install_tag_sync(&tx)?;
    }
    validate_integrity(&tx)?;
    tx.commit()?;
    Ok(())
}

fn upgrade(connection: &mut Connection, path: &Path) -> Result<()> {
    with_foreign_keys_restored(connection, |connection| {
        upgrade_transaction(connection, path)
    })
}

/// SQLite cannot change foreign_keys inside a transaction. Temporarily disable
/// it only on this exclusively leased migration connection, not on any reader.
/// Both initialization and upgrades check FK integrity before committing DDL.
fn with_foreign_keys_restored<T>(
    connection: &mut Connection,
    action: impl FnOnce(&mut Connection) -> Result<T>,
) -> Result<T> {
    let previous: bool = connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    connection.pragma_update(None, "foreign_keys", false)?;
    let result = action(connection);
    let restored = connection.pragma_update(None, "foreign_keys", previous);
    // Restore even when the transaction or a fault checkpoint failed.
    match result {
        Ok(value) => {
            restored?;
            Ok(value)
        }
        Err(error) => {
            restored?;
            Err(error)
        }
    }
}

fn upgrade_transaction(connection: &mut Connection, path: &Path) -> Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let DatabaseState::Version(version) = database_state(&tx, false)? else {
        return Err(Error::UnsupportedDatabase);
    };
    if version == 1 {
        validate_v1(&tx)?;
    } else {
        validate_history_at(&tx, version)?;
    }
    validate_integrity(&tx)?;
    check_available_space(&tx, path)?;
    let parent = path
        .parent()
        .ok_or(Error::InvalidInput("missing database parent"))?;
    let snapshot_dir = parent.join("migration-backups");
    std::fs::create_dir_all(&snapshot_dir)?;
    let filename = path
        .file_name()
        .ok_or(Error::InvalidInput("missing database filename"))?
        .to_string_lossy();
    let destination =
        snapshot_dir.join(format!("{filename}.v{version}-{}.sqlite3", Uuid::new_v4()));
    // A separate reader is required: backing up the connection with an active
    // write transaction yields SQLITE_LOCKED. BEGIN IMMEDIATE prevents all other
    // SQLite writers, including older executables that do not know our lease.
    let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(Duration::from_secs(5))?;
    snapshot::backup_to(&source, &destination)?;
    drop(source);
    #[cfg(test)]
    checkpoint(Checkpoint::AfterSnapshot, &tx)?;
    if version == 1 {
        install_workspace_metadata(&tx)?;
    }
    if version < 3 {
        install_rich_notes(&tx)?;
    }
    if version < 4 {
        install_attachments(&tx)?;
    }
    if version < 5 {
        install_note_queries(&tx)?;
    }
    if version < 6 {
        install_organization(&tx)?;
    }
    if version < 7 {
        install_collections_views(&tx)?;
    }
    if version < 8 {
        install_portable_jobs(&tx)?;
    }
    if version < 9 {
        install_sources(&tx)?;
    }
    if version < 10 {
        install_web_captures(&tx)?;
    }
    if version < 11 {
        install_source_titles(&tx)?;
    }
    if version < 12 {
        install_retrieval(&tx)?;
    }
    if version < 13 {
        install_review(&tx)?;
    }
    if version < 14 {
        install_chat(&tx)?;
    }
    if version < 15 {
        install_rich_format(&tx)?;
    }
    if version < 16 {
        install_generation_channels(&tx)?;
    }
    if version < 17 {
        install_reading_view(&tx)?;
    }
    if version < 18 {
        install_retrieval_params(&tx)?;
    }
    if version < 19 {
        install_usage_metrics(&tx)?;
    }
    if version < 20 {
        install_docx_format(&tx)?;
    }
    if version < 21 {
        install_source_author(&tx)?;
    }
    if version < 22 {
        install_text_cleaning(&tx)?;
    }
    if version < 23 {
        install_chunk_settings(&tx)?;
    }
    if version < 24 {
        install_model_call_latency(&tx)?;
    }
    if version < 25 {
        install_views_canvas(&tx)?;
    }
    if version < 26 {
        install_automation(&tx)?;
    }
    if version < 27 {
        install_external_audit(&tx)?;
    }
    if version < 28 {
        install_epub_format(&tx)?;
    }
    if version < 29 {
        install_spreadsheet_formats(&tx)?;
    }
    if version < 30 {
        install_pptx_format(&tx)?;
    }
    if version < 31 {
        install_mobi_format(&tx)?;
    }
    if version < 32 {
        install_citation_styles(&tx)?;
    }
    if version < 33 {
        install_block_database(&tx)?;
    }
    if version < 34 {
        install_source_book_navigation(&tx)?;
    }
    if version < 35 {
        install_source_images(&tx)?;
    }
    if version < 36 {
        install_xls_format(&tx)?;
    }
    if version < 37 {
        install_doc_format(&tx)?;
    }
    if version < 38 {
        install_workspace_metadata_v38(&tx)?;
    }
    if version < 39 {
        install_voice_captures(&tx)?;
    }
    if version < 40 {
        install_voice_segments(&tx)?;
    }
    if version < 41 {
        install_voice_sync_tasks(&tx)?;
    }
    if version < 42 {
        install_tag_management(&tx)?;
    }
    if version < 43 {
        install_knowledge_tasks(&tx)?;
    }
    if version < 44 {
        install_tag_sync(&tx)?;
    }
    #[cfg(test)]
    checkpoint(Checkpoint::AfterMetadata, &tx)?;
    #[cfg(test)]
    checkpoint(Checkpoint::BeforeCommit, &tx)?;
    validate_integrity(&tx)?;
    validate_history(&tx)?;
    tx.commit()?;
    Ok(())
}

fn install_reading_view(connection: &Connection) -> Result<()> {
    connection.execute_batch(READING_VIEW_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(17,?1,?2)",
        params![checksum(READING_VIEW_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=17", [])?;
    connection.pragma_update(None, "user_version", 17)?;
    Ok(())
}

fn install_docx_format(connection: &Connection) -> Result<()> {
    connection.execute_batch(DOCX_FORMAT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(20,?1,?2)",
        params![checksum(DOCX_FORMAT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=20", [])?;
    connection.pragma_update(None, "user_version", 20)?;
    Ok(())
}

fn install_epub_format(connection: &Connection) -> Result<()> {
    connection.execute_batch(EPUB_FORMAT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(28,?1,?2)",
        params![checksum(EPUB_FORMAT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=28", [])?;
    connection.pragma_update(None, "user_version", 28)?;
    Ok(())
}
fn install_spreadsheet_formats(connection: &Connection) -> Result<()> {
    connection.execute_batch(SPREADSHEET_FORMATS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(29,?1,?2)",
        params![
            checksum(SPREADSHEET_FORMATS_MIGRATION),
            Utc::now().to_rfc3339()
        ],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=29", [])?;
    connection.pragma_update(None, "user_version", 29)?;
    Ok(())
}
fn install_pptx_format(connection: &Connection) -> Result<()> {
    connection.execute_batch(PPTX_FORMAT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(30,?1,?2)",
        params![checksum(PPTX_FORMAT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=30", [])?;
    connection.pragma_update(None, "user_version", 30)?;
    Ok(())
}
fn install_mobi_format(connection: &Connection) -> Result<()> {
    connection.execute_batch(MOBI_FORMAT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(31,?1,?2)",
        params![checksum(MOBI_FORMAT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=31", [])?;
    connection.pragma_update(None, "user_version", 31)?;
    Ok(())
}

fn install_citation_styles(connection: &Connection) -> Result<()> {
    connection.execute_batch(CITATION_STYLES_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(32,?1,?2)",
        params![checksum(CITATION_STYLES_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=32", [])?;
    connection.pragma_update(None, "user_version", 32)?;
    Ok(())
}
fn install_block_database(connection: &Connection) -> Result<()> {
    let mut columns = connection
        .prepare("PRAGMA table_info(blocks)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<std::collections::BTreeSet<_>, _>>()?;
    for (name, declaration) in [
        ("revision_id", "TEXT"),
        ("parent_block_id", "TEXT"),
        ("position", "INTEGER"),
        ("depth", "INTEGER"),
        ("kind", "TEXT"),
        ("text", "TEXT"),
        ("block_alias", "TEXT"),
        ("updated_at", "TEXT"),
    ] {
        if columns.insert(name.to_owned()) {
            // Names and declarations are fixed constants; no user input reaches DDL.
            connection.execute_batch(&format!(
                "ALTER TABLE blocks ADD COLUMN {name} {declaration}"
            ))?;
        }
    }
    connection.execute_batch(BLOCK_DATABASE_MIGRATION)?;
    let notes = {
        let mut statement = connection.prepare(
            "SELECT n.id,n.current_revision_id,r.document_json,n.updated_at FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id ORDER BY n.id",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (note_id, revision_id, document_json, updated_at) in notes {
        let document: crate::NoteDocument = serde_json::from_str(&document_json)?;
        crate::blocks::replace_block_metadata(
            connection,
            &note_id,
            &revision_id,
            &document,
            &updated_at,
        )?;
    }
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(33,?1,?2)",
        params![checksum(BLOCK_DATABASE_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=33", [])?;
    connection.pragma_update(None, "user_version", 33)?;
    Ok(())
}

fn install_source_book_navigation(connection: &Connection) -> Result<()> {
    connection.execute_batch(SOURCE_BOOK_NAVIGATION_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(34,?1,?2)",
        params![
            checksum(SOURCE_BOOK_NAVIGATION_MIGRATION),
            Utc::now().to_rfc3339()
        ],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=34", [])?;
    connection.pragma_update(None, "user_version", 34)?;
    Ok(())
}
fn install_source_images(connection: &Connection) -> Result<()> {
    connection.execute_batch(SOURCE_IMAGES_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(35,?1,?2)",
        params![checksum(SOURCE_IMAGES_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=35", [])?;
    connection.pragma_update(None, "user_version", 35)?;
    Ok(())
}
fn install_xls_format(connection: &Connection) -> Result<()> {
    connection.execute_batch(XLS_FORMAT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(36,?1,?2)",
        params![checksum(XLS_FORMAT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=36", [])?;
    connection.pragma_update(None, "user_version", 36)?;
    Ok(())
}
fn install_workspace_metadata_v38(connection: &Connection) -> Result<()> {
    connection.execute_batch(WORKSPACE_METADATA_MIGRATION)?;
    let notes = {
        let mut statement = connection.prepare(
            "SELECT n.id,r.document_json FROM notes n
             JOIN note_revisions r ON r.revision_id=n.current_revision_id
             ORDER BY n.id",
        )?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (note_id, document_json) in notes {
        let document: crate::NoteDocument = serde_json::from_str(&document_json)?;
        connection.execute(
            "UPDATE notes SET type_key=?2 WHERE id=?1",
            params![note_id, crate::queries::frontmatter_type_key(&document)],
        )?;
    }
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(38,?1,?2)",
        params![
            checksum(WORKSPACE_METADATA_MIGRATION),
            Utc::now().to_rfc3339()
        ],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=38", [])?;
    connection.pragma_update(None, "user_version", 38)?;
    Ok(())
}
fn install_voice_captures(connection: &Connection) -> Result<()> {
    connection.execute_batch(VOICE_CAPTURES_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(39,?1,?2)",
        params![checksum(VOICE_CAPTURES_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=39", [])?;
    connection.pragma_update(None, "user_version", 39)?;
    Ok(())
}
fn install_voice_segments(connection: &Connection) -> Result<()> {
    connection.execute_batch(VOICE_SEGMENTS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(40,?1,?2)",
        params![checksum(VOICE_SEGMENTS_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=40", [])?;
    connection.pragma_update(None, "user_version", 40)?;
    Ok(())
}
fn install_tag_management(connection: &Connection) -> Result<()> {
    connection.execute_batch(TAG_MANAGEMENT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(42,?1,?2)",
        params![checksum(TAG_MANAGEMENT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=42", [])?;
    connection.pragma_update(None, "user_version", 42)?;
    Ok(())
}

fn install_tag_sync(connection: &Connection) -> Result<()> {
    connection.execute_batch(TAG_SYNC_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(44,?1,?2)",
        params![checksum(TAG_SYNC_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=44", [])?;
    connection.pragma_update(None, "user_version", 44)?;
    Ok(())
}

fn install_knowledge_tasks(connection: &Connection) -> Result<()> {
    connection.execute_batch(KNOWLEDGE_TASKS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(43,?1,?2)",
        params![checksum(KNOWLEDGE_TASKS_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=43", [])?;
    connection.pragma_update(None, "user_version", 43)?;
    Ok(())
}

fn install_voice_sync_tasks(connection: &Connection) -> Result<()> {
    connection.execute_batch(VOICE_SYNC_TASKS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(41,?1,?2)",
        params![
            checksum(VOICE_SYNC_TASKS_MIGRATION),
            Utc::now().to_rfc3339()
        ],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=41", [])?;
    connection.pragma_update(None, "user_version", 41)?;
    Ok(())
}
fn install_doc_format(connection: &Connection) -> Result<()> {
    connection.execute_batch(DOC_FORMAT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(37,?1,?2)",
        params![checksum(DOC_FORMAT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=37", [])?;
    connection.pragma_update(None, "user_version", 37)?;
    Ok(())
}
fn install_model_call_latency(connection: &Connection) -> Result<()> {
    connection.execute_batch(MODEL_CALL_LATENCY_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(24,?1,?2)",
        params![
            checksum(MODEL_CALL_LATENCY_MIGRATION),
            Utc::now().to_rfc3339()
        ],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=24", [])?;
    connection.pragma_update(None, "user_version", 24)?;
    Ok(())
}

fn install_views_canvas(connection: &Connection) -> Result<()> {
    connection.execute_batch(VIEWS_CANVAS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(25,?1,?2)",
        params![checksum(VIEWS_CANVAS_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=25", [])?;
    connection.pragma_update(None, "user_version", 25)?;
    Ok(())
}
fn install_automation(connection: &Connection) -> Result<()> {
    connection.execute_batch(AUTOMATION_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(26,?1,?2)",
        params![checksum(AUTOMATION_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=26", [])?;
    connection.pragma_update(None, "user_version", 26)?;
    Ok(())
}
fn install_external_audit(connection: &Connection) -> Result<()> {
    connection.execute_batch(EXTERNAL_AUDIT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(27,?1,?2)",
        params![checksum(EXTERNAL_AUDIT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=27", [])?;
    connection.pragma_update(None, "user_version", 27)?;
    Ok(())
}
fn install_chunk_settings(connection: &Connection) -> Result<()> {
    connection.execute_batch(CHUNK_SETTINGS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(23,?1,?2)",
        params![checksum(CHUNK_SETTINGS_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=23", [])?;
    connection.pragma_update(None, "user_version", 23)?;
    Ok(())
}

fn install_text_cleaning(connection: &Connection) -> Result<()> {
    connection.execute_batch(TEXT_CLEANING_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(22,?1,?2)",
        params![checksum(TEXT_CLEANING_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=22", [])?;
    connection.pragma_update(None, "user_version", 22)?;
    Ok(())
}

fn install_source_author(connection: &Connection) -> Result<()> {
    connection.execute_batch(SOURCE_AUTHOR_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(21,?1,?2)",
        params![checksum(SOURCE_AUTHOR_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=21", [])?;
    connection.pragma_update(None, "user_version", 21)?;
    Ok(())
}

fn install_usage_metrics(connection: &Connection) -> Result<()> {
    connection.execute_batch(USAGE_METRICS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(19,?1,?2)",
        params![checksum(USAGE_METRICS_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=19", [])?;
    connection.pragma_update(None, "user_version", 19)?;
    Ok(())
}

fn install_retrieval_params(connection: &Connection) -> Result<()> {
    connection.execute_batch(RETRIEVAL_PARAMS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(18,?1,?2)",
        params![
            checksum(RETRIEVAL_PARAMS_MIGRATION),
            Utc::now().to_rfc3339()
        ],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=18", [])?;
    connection.pragma_update(None, "user_version", 18)?;
    Ok(())
}

fn install_generation_channels(connection: &Connection) -> Result<()> {
    connection.execute_batch(GENERATION_CHANNELS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(16,?1,?2)",
        params![
            checksum(GENERATION_CHANNELS_MIGRATION),
            Utc::now().to_rfc3339()
        ],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=16", [])?;
    connection.pragma_update(None, "user_version", 16)?;
    Ok(())
}

fn install_rich_format(connection: &Connection) -> Result<()> {
    connection.execute_batch(RICH_FORMAT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(15,?1,?2)",
        params![checksum(RICH_FORMAT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=15", [])?;
    connection.pragma_update(None, "user_version", 15)?;
    Ok(())
}

fn install_chat(connection: &Connection) -> Result<()> {
    connection.execute_batch(CHAT_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(14,?1,?2)",
        params![checksum(CHAT_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=14", [])?;
    connection.pragma_update(None, "user_version", 14)?;
    Ok(())
}

fn install_review(connection: &Connection) -> Result<()> {
    connection.execute_batch(REVIEW_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(13,?1,?2)",
        params![checksum(REVIEW_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=13", [])?;
    connection.pragma_update(None, "user_version", 13)?;
    Ok(())
}

fn install_workspace_metadata(connection: &Connection) -> Result<()> {
    connection.execute_batch(WORKSPACE_MIGRATION)?;
    connection.execute(
        "INSERT INTO workspace_meta(singleton,workspace_id,schema_version,next_change_seq) VALUES(1,?1,?2,1)",
        params![Uuid::new_v4().to_string(), 2],
    )?;
    let now = Utc::now().to_rfc3339();
    for (version, sql) in [(1, BASELINE), (2, WORKSPACE_MIGRATION)] {
        connection.execute(
            "INSERT INTO migration_history(version,checksum,applied_at) VALUES(?1,?2,?3)",
            params![version, checksum(sql), now],
        )?;
    }
    connection.pragma_update(None, "user_version", 2)?;
    Ok(())
}

fn install_sources(connection: &Connection) -> Result<()> {
    connection.execute_batch(SOURCES_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(9,?1,?2)",
        params![checksum(SOURCES_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=9", [])?;
    connection.pragma_update(None, "user_version", 9)?;
    Ok(())
}

fn install_web_captures(connection: &Connection) -> Result<()> {
    connection.execute_batch(WEB_CAPTURES_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(10,?1,?2)",
        params![checksum(WEB_CAPTURES_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=10", [])?;
    connection.pragma_update(None, "user_version", 10)?;
    Ok(())
}

fn install_source_titles(connection: &Connection) -> Result<()> {
    connection.execute_batch(SOURCE_TITLES_MIGRATION)?;
    let titles = connection
        .prepare("SELECT id,display_title FROM sources")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, title) in titles {
        connection.execute(
            "INSERT INTO source_title_fts(source_id,title_tokens) VALUES(?1,?2)",
            params![id, crate::tokenize(&title)],
        )?;
    }
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(11,?1,?2)",
        params![checksum(SOURCE_TITLES_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=11", [])?;
    connection.pragma_update(None, "user_version", 11)?;
    Ok(())
}
fn install_retrieval(connection: &Connection) -> Result<()> {
    connection.execute_batch(RETRIEVAL_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(12,?1,?2)",
        params![checksum(RETRIEVAL_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=12", [])?;
    connection.pragma_update(None, "user_version", 12)?;
    Ok(())
}

fn install_rich_notes(connection: &Connection) -> Result<()> {
    connection.execute_batch(RICH_NOTES_MIGRATION)?;
    let rows = connection.prepare("SELECT note_id,revision,title,schema_version,document_json,created_at FROM legacy_note_revisions ORDER BY note_id,revision")?
        .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?,r.get::<_,String>(2)?,r.get::<_,u32>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?)))?
        .collect::<std::result::Result<Vec<_>,_>>()?;
    let mut parents = std::collections::BTreeMap::<String, String>::new();
    for (note, ordinal, title, schema, json, created) in rows {
        let id = Uuid::new_v4().to_string();
        let document: crate::NoteDocument = serde_json::from_str(&json)?;
        document.validate(schema)?;
        connection.execute("INSERT INTO note_revisions(revision_id,note_id,revision,parent_revision_id,title,schema_version,document_json,content_hash,author_kind,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'legacy',?9)", params![id,note,ordinal,parents.get(&note),title,schema,json,crate::notes::content_hash(&title,schema,&json),created])?;
        for node in document
            .nodes()
            .into_iter()
            .filter(|n| !n.attrs.id.is_empty())
        {
            let owner: Option<String> = connection
                .query_row(
                    "SELECT note_id FROM blocks WHERE id=?1",
                    [&node.attrs.id],
                    |r| r.get(0),
                )
                .optional()?;
            if owner.as_ref().is_some_and(|owner| owner != &note) {
                return Err(Error::IntegrityFailure);
            }
            connection.execute(
                "INSERT INTO blocks(id,note_id,active) VALUES(?1,?2,0) ON CONFLICT(id) DO NOTHING",
                params![node.attrs.id, note],
            )?;
            connection.execute(
                "INSERT INTO note_revision_blocks(revision_id,block_id) VALUES(?1,?2)",
                params![id, node.attrs.id],
            )?;
        }
        parents.insert(note, id);
    }
    connection.execute_batch("UPDATE notes SET current_revision_id=(SELECT revision_id FROM note_revisions WHERE note_id=notes.id AND revision=notes.current_revision); UPDATE blocks SET active=1 WHERE id IN (SELECT b.block_id FROM note_revision_blocks b JOIN notes n ON n.current_revision_id=b.revision_id); DROP TABLE legacy_note_revisions;")?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(3,?1,?2)",
        params![checksum(RICH_NOTES_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=3", [])?;
    connection.pragma_update(None, "user_version", 3)?;
    Ok(())
}

fn install_attachments(connection: &Connection) -> Result<()> {
    connection.execute_batch(ATTACHMENTS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(4,?1,?2)",
        params![checksum(ATTACHMENTS_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=4", [])?;
    connection.pragma_update(None, "user_version", 4)?;
    Ok(())
}

fn install_note_queries(connection: &Connection) -> Result<()> {
    connection.execute_batch(NOTE_QUERIES_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(5,?1,?2)",
        params![checksum(NOTE_QUERIES_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=5", [])?;
    connection.pragma_update(None, "user_version", 5)?;
    Ok(())
}

fn install_organization(connection: &Connection) -> Result<()> {
    connection.execute_batch(ORGANIZATION_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(6,?1,?2)",
        params![checksum(ORGANIZATION_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=6", [])?;
    connection.pragma_update(None, "user_version", 6)?;
    Ok(())
}

pub(crate) fn validate_history(connection: &Connection) -> Result<()> {
    validate_history_at(connection, CURRENT_SCHEMA_VERSION)
}

fn install_collections_views(connection: &Connection) -> Result<()> {
    connection.execute_batch(COLLECTIONS_VIEWS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(7,?1,?2)",
        params![
            checksum(COLLECTIONS_VIEWS_MIGRATION),
            Utc::now().to_rfc3339()
        ],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=7", [])?;
    connection.pragma_update(None, "user_version", 7)?;
    Ok(())
}

pub(crate) fn validate_history_at(connection: &Connection, expected_version: i64) -> Result<()> {
    let mut statement =
        connection.prepare("SELECT version,checksum FROM migration_history ORDER BY version")?;
    let records = statement
        .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (version, sql) in MIGRATIONS
        .into_iter()
        .filter(|(v, _)| *v <= expected_version)
    {
        if records
            .iter()
            .find(|record| i64::from(record.0) == version)
            .map(|record| &record.1)
            != Some(&checksum(sql))
        {
            return Err(Error::MigrationHistoryMismatch {
                version: version as u32,
            });
        }
    }
    if records.len() != expected_version as usize {
        return Err(Error::UnsupportedDatabase);
    }
    let (id, version, next): (String, i64, i64) = connection.query_row(
        "SELECT workspace_id,schema_version,next_change_seq FROM workspace_meta WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if Uuid::parse_str(&id).is_err() || version != expected_version || next < 1 {
        return Err(Error::UnsupportedDatabase);
    }
    Ok(())
}

fn install_portable_jobs(connection: &Connection) -> Result<()> {
    connection.execute_batch(PORTABLE_JOBS_MIGRATION)?;
    connection.execute(
        "INSERT INTO migration_history(version,checksum,applied_at) VALUES(8,?1,?2)",
        params![checksum(PORTABLE_JOBS_MIGRATION), Utc::now().to_rfc3339()],
    )?;
    connection.execute("UPDATE workspace_meta SET schema_version=8", [])?;
    connection.pragma_update(None, "user_version", 8)?;
    Ok(())
}

fn validate_v1(connection: &Connection) -> Result<()> {
    // Validate the released baseline without rewriting JSON or generating values
    // for user rows. Also reject half-applied or foreign metadata tables.
    let reserved: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name IN ('workspace_meta','migration_history')",
        [],
        |r| r.get(0),
    )?;
    if reserved != 0 {
        return Err(Error::UnsupportedDatabase);
    }
    for query in [
        "SELECT id,title,current_revision,anchor_id,trashed,updated_at FROM notes LIMIT 0",
        "SELECT note_id,revision,title,schema_version,document_json,created_at FROM note_revisions LIMIT 0",
        "SELECT id,name FROM collections LIMIT 0",
        "SELECT collection_id,note_id FROM memberships LIMIT 0",
        "SELECT source_id,target_id,context FROM note_links LIMIT 0",
        "SELECT note_id,title_tokens,body_tokens FROM note_fts LIMIT 0",
    ] {
        connection.prepare(query)?;
    }
    let broken: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM notes n LEFT JOIN note_revisions r ON r.note_id=n.id AND r.revision=n.current_revision WHERE r.note_id IS NULL OR r.title != n.title) OR EXISTS(SELECT 1 FROM note_revisions WHERE schema_version != 1)",
        [],
        |r| r.get(0),
    )?;
    if broken {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}

fn validate_integrity(connection: &Connection) -> Result<()> {
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(Error::IntegrityFailure);
    }
    if connection
        .prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}

fn check_available_space(connection: &Connection, path: &Path) -> Result<()> {
    #[cfg(test)]
    if matches!(FAULT.get(), Some(Fault::SpaceShortage)) {
        FAULT.set(None);
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "test-only preflight space shortage",
        )
        .into());
    }
    let parent = path
        .parent()
        .ok_or(Error::InvalidInput("missing database parent"))?;
    let available = fs2::available_space(parent)?;
    require_space(database_bytes(connection, path)?, available)
}

/// Logical pages plus committed WAL frames. A snapshot copies both, so a
/// preflight based on the main file alone would under-estimate a busy library.
fn database_bytes(connection: &Connection, path: &Path) -> Result<u64> {
    let pages: i64 = connection.pragma_query_value(None, "page_count", |r| r.get(0))?;
    let page_size: i64 = connection.pragma_query_value(None, "page_size", |r| r.get(0))?;
    let logical = u64::try_from(pages)
        .map_err(|_| Error::IntegrityFailure)?
        .saturating_mul(u64::try_from(page_size).map_err(|_| Error::IntegrityFailure)?);
    let main = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut wal_name = path.as_os_str().to_os_string();
    wal_name.push("-wal");
    let wal = std::fs::metadata(PathBuf::from(wal_name))
        .map(|m| m.len())
        .unwrap_or(0);
    Ok(logical.max(main).saturating_add(wal))
}

fn require_space(database_bytes: u64, available: u64) -> Result<()> {
    // One complete snapshot plus conservative journal/upgrade headroom. This is
    // an early check, not a disk reservation: every later I/O error still aborts.
    let required = database_bytes.saturating_mul(2).saturating_add(1024 * 1024);
    if available < required {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "insufficient space for migration snapshot and journal",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Checkpoint {
    AfterSnapshot,
    AfterMetadata,
    BeforeCommit,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum Fault {
    Fail(Checkpoint),
    Exit(Checkpoint),
    DiskFull,
    SpaceShortage,
}

#[cfg(test)]
thread_local! {
    static FAULT: std::cell::Cell<Option<Fault>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn checkpoint(point: Checkpoint, connection: &Connection) -> Result<()> {
    match FAULT.get() {
        Some(Fault::Fail(target)) if target == point => {
            FAULT.set(None);
            // A real SQLite statement failure after preceding migration writes.
            connection.execute_batch("INSERT INTO pkms_test_nonexistent_table VALUES(1)")?;
            unreachable!("the test-only failure statement must not succeed")
        }
        Some(Fault::Exit(target)) if target == point => std::process::exit(86),
        Some(Fault::DiskFull) if point == Checkpoint::AfterSnapshot => {
            FAULT.set(None);
            let pages: i64 = connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
            connection.pragma_update(None, "max_page_count", pages)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Seek, SeekFrom, Write},
        process::{Child, Command, Stdio},
        time::Instant,
    };
    use tempfile::tempdir;

    fn fixture(path: &Path) {
        Connection::open(path)
            .unwrap()
            .execute_batch(include_str!("../tests/fixtures/schema-v1.sql"))
            .unwrap();
    }

    fn version(path: &Path) -> i64 {
        Connection::open(path)
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap()
    }

    fn snapshots(path: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(path.parent().unwrap().join("migration-backups"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "sqlite3")
            })
            .collect()
    }

    fn check_rolled_back(path: &Path) {
        let connection = Connection::open(path).unwrap();
        assert_eq!(version(path), 1);
        let count: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name IN ('workspace_meta','migration_history')",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 0);
        validate_integrity(&connection).unwrap();
        let original = Connection::open_in_memory().unwrap();
        original
            .execute_batch(include_str!("../tests/fixtures/schema-v1.sql"))
            .unwrap();
        for query in [
            "SELECT hex(document_json) FROM note_revisions ORDER BY note_id,revision",
            "SELECT id||title||anchor_id||current_revision||trashed FROM notes ORDER BY id",
            "SELECT collection_id||note_id FROM memberships ORDER BY collection_id,note_id",
            "SELECT source_id||target_id||context FROM note_links ORDER BY source_id,target_id",
        ] {
            let collect = |connection: &Connection| -> Vec<String> {
                connection
                    .prepare(query)
                    .unwrap()
                    .query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .unwrap()
            };
            assert_eq!(collect(&connection), collect(&original));
        }
    }

    #[test]
    fn migration_statement_failure_rolls_back_all_ddl_and_retry_keeps_prior_snapshot() {
        for point in [
            Checkpoint::AfterSnapshot,
            Checkpoint::AfterMetadata,
            Checkpoint::BeforeCommit,
        ] {
            let dir = tempdir().unwrap();
            let path = dir.path().join("library.sqlite3");
            fixture(&path);
            FAULT.set(Some(Fault::Fail(point)));
            assert!(matches!(
                open_database(&path, false),
                Err(Error::Database(_))
            ));
            check_rolled_back(&path);
            let first_snapshots = snapshots(&path);
            assert_eq!(first_snapshots.len(), 1);
            let original_snapshot = std::fs::read(&first_snapshots[0]).unwrap();
            assert_eq!(version(&first_snapshots[0]), 1);
            drop(open_database(&path, false).unwrap());
            assert_eq!(version(&path), CURRENT_SCHEMA_VERSION);
            assert_eq!(snapshots(&path).len(), 2);
            assert_eq!(
                std::fs::read(&first_snapshots[0]).unwrap(),
                original_snapshot
            );
        }
    }

    #[test]
    fn disk_preflight_rejects_shortage_and_arithmetic_overflow() {
        assert!(
            matches!(require_space(4096, 1024), Err(Error::Io(error)) if error.kind() == io::ErrorKind::StorageFull)
        );
        assert!(require_space(4096, 4096 * 2 + 1024 * 1024).is_ok());
        assert!(require_space(u64::MAX, u64::MAX - 1).is_err());
    }

    #[test]
    fn space_preflight_shortage_aborts_before_snapshot_and_can_retry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        fixture(&path);
        let before = std::fs::read(&path).unwrap();
        FAULT.set(Some(Fault::SpaceShortage));
        assert!(matches!(
            open_database(&path, false),
            Err(Error::Io(error)) if error.kind() == io::ErrorKind::StorageFull
        ));
        check_rolled_back(&path);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.path().join("migration-backups").exists());
        drop(open_database(&path, false).unwrap());
        assert_eq!(version(&path), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn space_estimate_counts_committed_wal_frames() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        fixture(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; \
                 CREATE TABLE space_probe(value BLOB); \
                 INSERT INTO space_probe VALUES(zeroblob(524288));",
            )
            .unwrap();
        let mut wal_name = path.as_os_str().to_os_string();
        wal_name.push("-wal");
        let wal = std::fs::metadata(PathBuf::from(wal_name)).unwrap().len();
        let main = std::fs::metadata(&path).unwrap().len();
        let estimate = database_bytes(&connection, &path).unwrap();
        assert!(wal > 0);
        assert!(
            estimate >= main.saturating_add(wal),
            "estimate {estimate} < main {main} + wal {wal}"
        );
    }

    #[test]
    fn corrupt_database_aborts_before_snapshot_without_changing_bytes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        fixture(&path);
        let (root, page_size) = {
            let connection = Connection::open(&path).unwrap();
            (
                connection
                    .query_row(
                        "SELECT rootpage FROM sqlite_schema WHERE name='notes'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                connection
                    .pragma_query_value(None, "page_size", |row| row.get::<_, i64>(0))
                    .unwrap(),
            )
        };
        let offset = u64::try_from((root - 1) * page_size).unwrap();
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(&vec![0u8; page_size as usize]).unwrap();
        file.sync_all().unwrap();
        drop(file);
        let before = std::fs::read(&path).unwrap();
        match open_database(&path, false) {
            Err(Error::IntegrityFailure | Error::Database(_)) => {}
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("a corrupt database must not migrate"),
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.path().join("migration-backups").exists());
    }

    #[test]
    fn snapshot_integrity_failure_aborts_before_ddl_and_can_retry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        fixture(&path);
        let before = std::fs::read(&path).unwrap();
        snapshot::force_integrity_failure();
        assert!(matches!(
            open_database(&path, false),
            Err(Error::IntegrityFailure)
        ));
        check_rolled_back(&path);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(snapshots(&path).is_empty());
        drop(open_database(&path, false).unwrap());
        assert_eq!(version(&path), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn sqlite_full_during_schema_write_preserves_original_and_can_retry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        fixture(&path);
        FAULT.set(Some(Fault::DiskFull));
        assert!(matches!(
            open_database(&path, false),
            Err(Error::Database(rusqlite::Error::SqliteFailure(error, _)))
                if error.code == rusqlite::ErrorCode::DiskFull
        ));
        check_rolled_back(&path);
        let original = snapshots(&path);
        assert_eq!(original.len(), 1);
        check_rolled_back(&original[0]);
        drop(open_database(&path, false).unwrap());
        assert_eq!(version(&path), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn pre_migration_snapshot_is_a_complete_rollback_point() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        fixture(&path);
        drop(open_database(&path, false).unwrap());
        let snapshot = snapshots(&path).into_iter().next().unwrap();
        // The snapshot is a complete v1 database with all original rows.
        check_rolled_back(&snapshot);
        let restored_dir = dir.path().join("restored");
        std::fs::create_dir_all(&restored_dir).unwrap();
        let restored = restored_dir.join("library.sqlite3");
        std::fs::copy(&snapshot, &restored).unwrap();
        check_rolled_back(&restored);
        // A restored copy is usable: the current app re-upgrades it to K24.
        drop(open_database(&restored, false).unwrap());
        assert_eq!(version(&restored), CURRENT_SCHEMA_VERSION);
        assert_eq!(version(&path), CURRENT_SCHEMA_VERSION);
    }
    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn child(path: &Path, mode: &str) -> ChildGuard {
        ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "migrations::tests::migration_process_child",
                    "--nocapture",
                ])
                .env("PKMS_TEST_MIGRATION_DATABASE", path)
                .env("PKMS_TEST_MIGRATION_MODE", mode)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    fn wait_ready(path: &Path, child: &mut ChildGuard) {
        let ready = path.with_extension("ready");
        let start = Instant::now();
        while !ready.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "lock child exited before handshake"
            );
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "lock child did not become ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn process_termination_at_each_checkpoint_preserves_recoverable_v1() {
        for mode in ["crash-snapshot", "crash-metadata", "crash-commit"] {
            let dir = tempdir().unwrap();
            let path = dir.path().join("library.sqlite3");
            fixture(&path);
            let mut process = child(&path, mode);
            assert_eq!(process.0.wait().unwrap().code(), Some(86));
            check_rolled_back(&path);
            let original_snapshots = snapshots(&path);
            assert_eq!(original_snapshots.len(), 1);
            check_rolled_back(&original_snapshots[0]);
            drop(open_database(&path, false).unwrap());
            assert_eq!(version(&path), CURRENT_SCHEMA_VERSION);
            assert_eq!(snapshots(&path).len(), 2);
        }
    }

    #[test]
    fn shared_lease_in_another_process_blocks_migration_and_releases_on_exit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        fixture(&path);
        let mut process = child(&path, "shared-v1");
        wait_ready(&path, &mut process);
        assert!(matches!(
            open_database(&path, false),
            Err(Error::WorkspaceBusy)
        ));
        assert_eq!(version(&path), 1);
        assert!(!dir.path().join("migration-backups").exists());
        drop(process);
        drop(open_database(&path, false).unwrap());
        assert_eq!(version(&path), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn current_library_leases_allow_readers_but_exclude_migration_across_processes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        drop(open_database(&path, true).unwrap());
        let mut process = child(&path, "shared-current");
        wait_ready(&path, &mut process);
        let second = open_database(&path, false).unwrap();
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path(&path))
            .unwrap();
        assert!(matches!(
            contender.try_lock(),
            Err(TryLockError::WouldBlock)
        ));
        drop(second);
        assert!(matches!(
            contender.try_lock(),
            Err(TryLockError::WouldBlock)
        ));
        drop(process);
        contender.try_lock().unwrap();
        assert!(matches!(
            open_database(&path, false),
            Err(Error::WorkspaceBusy)
        ));
        contender.unlock().unwrap();
        assert!(open_database(&path, false).is_ok());
    }

    // Only the unit-test executable has these inputs and interruption points.
    // Ordinary cargo test calls this with no environment and it immediately exits.
    #[test]
    fn migration_process_child() {
        let Some(path) = std::env::var_os("PKMS_TEST_MIGRATION_DATABASE") else {
            return;
        };
        let path = PathBuf::from(path);
        match std::env::var("PKMS_TEST_MIGRATION_MODE").unwrap().as_str() {
            "crash-snapshot" | "crash-metadata" | "crash-commit" => {
                let mode = std::env::var("PKMS_TEST_MIGRATION_MODE").unwrap();
                let point = match mode.as_str() {
                    "crash-snapshot" => Checkpoint::AfterSnapshot,
                    "crash-metadata" => Checkpoint::AfterMetadata,
                    _ => Checkpoint::BeforeCommit,
                };
                FAULT.set(Some(Fault::Exit(point)));
                let _ = open_database(&path, false);
                panic!("migration did not reach the requested interruption point");
            }
            "shared-v1" => {
                let lock = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(lock_path(&path))
                    .unwrap();
                lock.try_lock_shared().unwrap();
                std::fs::write(path.with_extension("ready"), b"ready").unwrap();
                std::thread::sleep(Duration::from_secs(30));
                drop(lock);
            }
            "shared-current" => {
                let opened = open_database(&path, false).unwrap();
                std::fs::write(path.with_extension("ready"), b"ready").unwrap();
                std::thread::sleep(Duration::from_secs(30));
                drop(opened);
            }
            mode => panic!("unexpected test mode: {mode}"),
        }
    }

    #[test]
    fn schema_three_upgrade_preserves_revision_and_receipt_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        let mut raw = Connection::open(&path).unwrap();
        raw.execute_batch(include_str!("../tests/fixtures/schema-v1.sql"))
            .unwrap();
        let tx = raw.transaction().unwrap();
        install_workspace_metadata(&tx).unwrap();
        install_rich_notes(&tx).unwrap();
        let note_id = "11111111-1111-4111-8111-111111111111";
        let revision_id: String = tx
            .query_row(
                "SELECT current_revision_id FROM notes WHERE id=?1",
                [note_id],
                |r| r.get(0),
            )
            .unwrap();
        let command_id = Uuid::new_v4().to_string();
        tx.execute(
        "INSERT INTO command_receipts VALUES(?1,?2,'notes.save',?3,?4,1,'2026-10-01T00:00:00Z')",
        params![command_id, "a".repeat(64), note_id, revision_id],
    )
    .unwrap();
        tx.execute("UPDATE workspace_meta SET next_change_seq=2", [])
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(version(&path), 3);
        drop(raw);
        let library = crate::Library::open_existing(dir.path()).unwrap();
        assert_eq!(library.note(note_id).unwrap().revision_id, revision_id);
        let raw = Connection::open(&path).unwrap();
        assert_eq!(
            raw.query_row(
                "SELECT revision_id FROM command_receipts WHERE command_id=?1",
                [command_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            revision_id
        );
        assert_eq!(
            raw.query_row("SELECT next_change_seq FROM workspace_meta", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        let backups = snapshots(&path);
        assert_eq!(backups.len(), 1);
        assert_eq!(version(&backups[0]), 3);
    }

    #[test]
    fn schema_five_upgrade_preserves_rich_history_images_collections_and_receipts() {
        use crate::{CreateNote, NoteDocument, SaveNote};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        // Construct the previous release using its unchanged published migrations.
        let mut connection = connect(&path, true).unwrap();
        let tx = connection.transaction().unwrap();
        tx.execute_batch(BASELINE).unwrap();
        install_workspace_metadata(&tx).unwrap();
        install_rich_notes(&tx).unwrap();
        install_attachments(&tx).unwrap();
        install_note_queries(&tx).unwrap();
        tx.commit().unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path(&path))
            .unwrap();
        shared_lock(&lock).unwrap();
        let mut old = crate::Library {
            connection,
            database_path: path.canonicalize().unwrap(),
            attachment_previews: Default::default(),
            _lock: lock,
        };
        old.connection
            .execute_batch("ALTER TABLE notes ADD COLUMN type_key TEXT;")
            .unwrap();
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(3, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let bytes = bytes.into_inner();
        let capture = old
            .import_attachment(&Uuid::new_v4().to_string(), "历史图片.png", &bytes)
            .unwrap();
        let request = CreateNote {
            collection_id: None,
            base_collection_row_version: None,
            title: "旧版富文本".into(),
            document_schema: 2,
            document: serde_json::from_value(serde_json::json!({
                "type":"doc", "content":[{"type":"image","attrs":{
                    "id":Uuid::new_v4().to_string(),"blob_hash":capture.blob.hash,"alt":"历史图片"
                }}]
            }))
            .unwrap(),
        };
        let create_id = Uuid::new_v4().to_string();
        let original = old.create_document(&create_id, request.clone()).unwrap();
        let saved = old
            .save_document(
                &Uuid::new_v4().to_string(),
                SaveNote {
                    note_id: original.note.id.clone(),
                    base_revision_id: original.note.revision_id.clone(),
                    title: "当前正文已删图片".into(),
                    document_schema: 2,
                    document: NoteDocument::from_text(
                        Uuid::new_v4().to_string(),
                        "历史必须继续保留图片",
                    ),
                },
            )
            .unwrap();
        let collection = crate::Collection {
            id: Uuid::new_v4().to_string(),
            name: "既有主题".into(),
        };
        old.connection
            .execute(
                "INSERT INTO collections(id,name) VALUES(?1,?2)",
                params![collection.id, collection.name],
            )
            .unwrap();
        old.connection
            .execute(
                "INSERT INTO objects(id,kind) VALUES(?1,'collection')",
                [&collection.id],
            )
            .unwrap();
        old.connection
            .execute(
                "INSERT INTO memberships(collection_id,note_id) VALUES(?1,?2)",
                params![collection.id, original.note.id],
            )
            .unwrap();
        // The frozen v5 revision table only accepts document schemas 1 and 2, so this
        // fixture writes an explicit D2 note instead of the D3 default.
        let other = old
            .create_document(
                &Uuid::new_v4().to_string(),
                CreateNote {
                    collection_id: None,
                    base_collection_row_version: None,
                    title: "来源笔记".into(),
                    document_schema: 2,
                    document: NoteDocument::from_text(Uuid::new_v4().to_string(), "正文"),
                },
            )
            .unwrap()
            .note;
        old.link_notes(&other.id, &original.note.id, "手工关系上下文")
            .unwrap();
        let old_info = old.info().unwrap();
        assert_eq!(old_info.database_schema, 5);
        let before_history = old.revisions(&original.note.id).unwrap();
        old.connection
            .execute_batch("ALTER TABLE notes DROP COLUMN type_key;")
            .unwrap();
        drop(old);

        let mut upgraded = crate::Library::open_existing(dir.path()).unwrap();
        let new_info = upgraded.info().unwrap();
        assert_eq!(new_info.database_schema, CURRENT_SCHEMA_VERSION as u32);
        assert_eq!(new_info.workspace_id, old_info.workspace_id);
        assert_eq!(&new_info.migrations[..5], old_info.migrations.as_slice());
        assert_eq!(upgraded.note(&original.note.id).unwrap(), saved.note);
        assert_eq!(
            upgraded.revisions(&original.note.id).unwrap(),
            before_history
        );
        assert_eq!(
            upgraded
                .attachment(&original.note.revision_id, &capture.blob.hash)
                .unwrap()
                .1,
            bytes
        );
        assert_eq!(
            upgraded.collection_notes(&collection.id).unwrap()[0].id,
            original.note.id
        );
        assert_eq!(
            upgraded.backlinks(&original.note.id).unwrap()[0].context,
            "手工关系上下文"
        );
        let replay = upgraded.create_document(&create_id, request).unwrap();
        assert!(replay.receipt.replayed);
        assert_eq!(replay.note, original.note);
        assert_eq!(replay.receipt.change_seq, original.receipt.change_seq);
        let metadata = upgraded.object_metadata(&original.note.id).unwrap();
        assert_eq!(metadata.inbox_state, Some(crate::InboxState::Pending));
        assert!(metadata.tags.is_empty());
        assert_eq!(metadata.row_version, 2);
        let backup = snapshots(&path);
        assert_eq!(backup.len(), 1);
        assert_eq!(version(&backup[0]), 5);
    }

    #[test]
    fn schema_eight_upgrade_retains_identity_and_adds_empty_source_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        let mut connection = connect(&path, true).unwrap();
        initialize_at(&mut connection, 8).unwrap();
        let identity: String = connection
            .query_row("SELECT workspace_id FROM workspace_meta", [], |r| r.get(0))
            .unwrap();
        drop(connection);
        let library = crate::Library::open_existing(dir.path()).unwrap();
        assert_eq!(library.info().unwrap().workspace_id, identity);
        assert_eq!(
            library.info().unwrap().database_schema,
            CURRENT_SCHEMA_VERSION as u32
        );
        assert_eq!(
            library
                .connection
                .query_row("SELECT count(*) FROM sources", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            0
        );
        assert_eq!(snapshots(&path).len(), 1);
        assert_eq!(version(&snapshots(&path)[0]), 8);
    }

    #[test]
    fn schema_six_upgrade_preserves_metadata_receipts_and_does_not_invent_collection_times() {
        use crate::{AssignTag, InboxState, MetadataTarget, SetInboxState, TagSelector};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        let mut connection = connect(&path, true).unwrap();
        connection
            .execute_batch(include_str!("../tests/fixtures/schema-v1.sql"))
            .unwrap();
        let tx = connection.transaction().unwrap();
        install_workspace_metadata(&tx).unwrap();
        install_rich_notes(&tx).unwrap();
        install_attachments(&tx).unwrap();
        install_note_queries(&tx).unwrap();
        install_organization(&tx).unwrap();
        tx.commit().unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path(&path))
            .unwrap();
        shared_lock(&lock).unwrap();
        let mut old = crate::Library {
            connection,
            database_path: path.canonicalize().unwrap(),
            attachment_previews: Default::default(),
            _lock: lock,
        };
        let note_id = "11111111-1111-4111-8111-111111111111";
        let collection_id = "44444444-4444-4444-8444-444444444444";
        let command_id = Uuid::new_v4().to_string();
        let request = AssignTag {
            targets: vec![MetadataTarget {
                object_id: note_id.into(),
                base_row_version: 1,
            }],
            tag: TagSelector::Name {
                name: "已保存标签".into(),
            },
        };
        let original = old.assign_tag(&command_id, request.clone()).unwrap();
        old.set_inbox_state(
            &Uuid::new_v4().to_string(),
            SetInboxState {
                object_id: note_id.into(),
                base_row_version: 2,
                target: InboxState::Processed,
            },
        )
        .unwrap();
        let metadata = old.object_metadata(note_id).unwrap();
        let old_info = old.info().unwrap();
        assert_eq!(old_info.database_schema, 6);
        let note = old.note(note_id).unwrap();
        drop(old);
        let mut upgraded = crate::Library::open_existing(dir.path()).unwrap();
        let info = upgraded.info().unwrap();
        assert_eq!(info.workspace_id, old_info.workspace_id);
        assert_eq!(&info.migrations[..6], old_info.migrations.as_slice());
        assert_eq!(upgraded.note(note_id).unwrap(), note);
        assert_eq!(upgraded.object_metadata(note_id).unwrap(), metadata);
        let collection = upgraded.collection_get(collection_id).unwrap();
        assert!(collection.created_at.is_none() && collection.updated_at.is_none());
        assert_eq!(collection.metadata.row_version, 1);
        // The legacy fixture has one live and one independently trashed member.
        assert_eq!(collection.member_count, 2);
        assert_eq!(
            upgraded.collection_notes(collection_id).unwrap()[0].id,
            note_id
        );
        let replay = upgraded.assign_tag(&command_id, request).unwrap();
        assert!(replay.receipt.replayed);
        assert_eq!(replay.objects, original.objects);
        assert_eq!(replay.receipt.change_seq, original.receipt.change_seq);
        let backup = snapshots(&path);
        assert_eq!(backup.len(), 1);
        assert_eq!(version(&backup[0]), 6);
    }
}

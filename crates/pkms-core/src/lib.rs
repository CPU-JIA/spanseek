//! Tauri-independent PKMS data core. G0 covers durable notes and organization.
mod annotation_queries;
mod annotation_retrieval;
mod knowledge_task_context;
mod knowledge_task_models;
mod knowledge_tasks;
mod task_templates;
pub use knowledge_task_models::*;
mod attachments;
mod block_models;
mod blocks;
pub use block_models::*;
mod collection_models;
mod collections;
pub use collection_models::*;
mod diagnostic_models;
mod diagnostics;
mod document;
pub use diagnostic_models::*;
pub use diagnostics::prepare_diagnostics_archive;
mod lease;
mod metrics;
pub use metrics::{ModelCallLatency, ModelUsage, UsagePurpose, duration_ms, elapsed_ms};
mod generation_timeouts;
mod graph;
mod handoff;
pub use generation_timeouts::{
    GenerationPurpose, default_timeout_seconds, embedding_timeout_seconds,
};
pub use graph::*;
mod migrations;
pub use lease::WorkspaceLease;
pub use migrations::CURRENT_SCHEMA_VERSION;
mod links;
mod notes;
pub use links::*;
mod knowledge_index;
mod knowledge_models;
mod organization;
mod organization_models;
mod tag_management;
mod tag_management_models;
mod tag_sync;
mod tag_sync_models;
pub use knowledge_models::*;
pub use tag_management_models::*;
pub use tag_sync::tag_sync_object_id;
pub use tag_sync_models::*;
mod queries;
pub use document::*;
use notes::read_note;
pub use organization_models::*;
mod chat;
mod chat_models;
pub use chat::validate_chat_run_options;
pub use chat_models::*;
mod assistant;
mod assistant_models;
mod automation;
#[cfg(test)]
#[path = "automation_fault_tests.rs"]
mod automation_fault_tests;
mod automation_models;
mod generation_channel_models;
mod generation_channels;
pub use generation_channel_models::*;
pub use generation_channels::validate_generation_channel_spec;
mod canvas_models;
mod canvases;
mod chinese;
mod chunk_settings;
mod citation;
mod cleaning;
mod export;
mod external_audit;
mod external_contract;
mod index;
mod index_jobs;
mod joplin;
mod models;
mod mutations;
mod obsidian;
mod portable;
mod portable_jobs;
mod portable_models;
mod portable_validation;
mod report;
mod report_models;
mod research;
mod retrieval;
mod retrieval_models;
mod retrieval_settings;
mod review;
mod review_models;
mod snapshot;
mod source_annotations;
mod source_jobs;
mod source_models;
mod source_notes;
mod source_tables;
mod sources;
mod statistics;
mod statistics_models;
mod trash;
mod trash_models;
pub use assistant::{validate_generation_profile, validate_structured_generation_profile};
pub use assistant_models::*;
pub use automation_models::*;
pub use canvas_models::*;
pub use chinese::*;
pub use chunk_settings::*;
pub use citation::*;
pub use cleaning::*;
pub use external_contract::*;
pub use joplin::*;
pub use obsidian::*;
pub use portable::{
    PreparedOutput, PreparedRestore, inspect_backup, prepare_restore, verify_backup,
};
pub use portable_jobs::*;
pub use portable_models::*;
pub use report_models::*;
pub use retrieval_models::*;
pub use review_models::*;
pub use source_models::*;
pub use sources::parse_text_source;
pub use statistics_models::*;
pub use trash_models::*;
mod view_models;
mod views;
mod voice;
mod voice_models;
mod wiki;
mod workspace_metadata;
mod workspace_metadata_models;
pub use models::*;
pub use view_models::*;
pub use voice_models::*;
pub use workspace_metadata_models::*;

use jieba_rs::Jieba;
use rusqlite::{Connection, params};
use std::{collections::BTreeSet, path::Path, sync::OnceLock};
use thiserror::Error;
use uuid::Uuid;

const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
static TOKENIZER: OnceLock<Jieba> = OnceLock::new();

#[derive(Debug, Error)]
pub enum Error {
    #[error("query cursor expired; reload the first page")]
    QueryExpired,
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("document encoding error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("object not found")]
    NotFound,
    #[error("revision conflict: expected {expected}, current {actual}")]
    Conflict { expected: u32, actual: u32 },
    #[error("revision conflict: expected {expected}, current {actual}")]
    RevisionConflict { expected: String, actual: String },
    #[error("metadata conflict for {object_id}: expected {expected}, current {actual}")]
    MetadataConflict {
        object_id: String,
        expected: u32,
        actual: u32,
    },
    #[error("command ID has already been used with a different request")]
    IdempotencyMismatch,
    #[error("invalid document: {0}")]
    InvalidDocument(&'static str),
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("unsupported or unrelated database")]
    UnsupportedDatabase,
    #[error("note is in the trash")]
    Trashed,
    #[error("backup did not finish within the time limit")]
    BackupTimeout,
    #[error("database integrity check failed")]
    IntegrityFailure,
    #[error("workspace is busy; close other connections before migrating")]
    WorkspaceBusy,
    #[error("migration history does not match the application for version {version}")]
    MigrationHistoryMismatch { version: u32 },
}
pub type Result<T> = std::result::Result<T, Error>;

pub struct Library {
    connection: Connection,
    database_path: std::path::PathBuf,
    attachment_previews: std::collections::HashMap<String, (String, std::time::Instant)>,
    // Close SQLite before releasing the shared workspace lock.
    _lock: std::fs::File,
}

impl Library {
    pub fn open(workspace: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(workspace.as_ref())?;
        Self::open_database(workspace.as_ref().join("library.sqlite3"))
    }

    /// Opens our database or initializes an empty path; supported old versions migrate first.
    pub fn open_database(path: impl AsRef<Path>) -> Result<Self> {
        let opened = migrations::open_database(path.as_ref(), true)?;
        let mut library = Self {
            database_path: path.as_ref().canonicalize()?,
            attachment_previews: Default::default(),
            connection: opened.connection,
            _lock: opened.lock,
        };
        library.recover_interrupted_voice_transcriptions()?;
        Ok(library)
    }

    /// Opens an existing workspace, migrating supported old schemas without creating typo paths.
    pub fn open_existing(workspace: impl AsRef<Path>) -> Result<Self> {
        let path = workspace.as_ref().join("library.sqlite3");
        let opened = migrations::open_database(&path, false)?;
        let mut library = Self {
            database_path: path.canonicalize()?,
            attachment_previews: Default::default(),
            connection: opened.connection,
            _lock: opened.lock,
        };
        library.recover_interrupted_voice_transcriptions()?;
        Ok(library)
    }

    pub fn create_note(&mut self, title: &str, body: &str) -> Result<Note> {
        validate_text(title, body)?;
        self.create_document(
            &Uuid::new_v4().to_string(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: title.into(),
                document_schema: DOCUMENT_SCHEMA,
                document: NoteDocument::from_text(Uuid::new_v4().to_string(), body),
            },
        )
        .map(|result| result.note)
    }

    pub fn info(&self) -> Result<WorkspaceInfo> {
        let (workspace_id, database_schema) = self.connection.query_row(
            "SELECT workspace_id,schema_version FROM workspace_meta",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
        )?;
        let mut statement = self.connection.prepare(
            "SELECT version,checksum,applied_at FROM migration_history ORDER BY version",
        )?;
        let migrations = statement
            .query_map([], |row| {
                Ok(MigrationRecord {
                    version: row.get(0)?,
                    checksum: row.get(1)?,
                    applied_at: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(WorkspaceInfo {
            workspace_id,
            database_schema,
            supported_document_schemas: SUPPORTED_DOCUMENT_SCHEMAS.to_vec(),
            migrations,
        })
    }

    pub fn note(&self, id: &str) -> Result<Note> {
        read_note(&self.connection, id)
    }

    pub fn contains_object(&self, id: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?)
    }

    /// The legacy text CLI never silently flattens an existing rich document.
    pub fn save_note(
        &mut self,
        id: &str,
        base_revision: u32,
        title: &str,
        body: &str,
    ) -> Result<Note> {
        validate_text(title, body)?;
        let current = self.note(id)?;
        if current.revision != base_revision {
            return Err(Error::Conflict {
                expected: base_revision,
                actual: current.revision,
            });
        }
        if current.document.validate(1).is_err() {
            return Err(Error::InvalidDocument(
                "use structured save to edit rich text",
            ));
        }
        let result = self.save_document(
            &Uuid::new_v4().to_string(),
            SaveNote {
                note_id: id.into(),
                base_revision_id: current.revision_id,
                title: title.into(),
                document_schema: DOCUMENT_SCHEMA,
                document: NoteDocument::from_text(
                    current.document.content[0].attrs.id.clone(),
                    body,
                ),
            },
        );
        match result {
            Err(Error::RevisionConflict { .. }) => Err(Error::Conflict {
                expected: base_revision,
                actual: self.note(id)?.revision,
            }),
            result => result.map(|result| result.note),
        }
    }

    pub fn revisions(&self, id: &str) -> Result<Vec<Note>> {
        self.note(id)?;
        let ids = self
            .connection
            .prepare("SELECT revision_id FROM note_revisions WHERE note_id=?1 ORDER BY revision")?
            .query_map([id], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter()
            .map(|revision_id| self.revision(id, revision_id))
            .collect()
    }

    pub fn restore_revision(
        &mut self,
        id: &str,
        base_revision: u32,
        revision: u32,
    ) -> Result<Note> {
        let current = self.note(id)?;
        if current.revision != base_revision {
            return Err(Error::Conflict {
                expected: base_revision,
                actual: current.revision,
            });
        }
        let target = self
            .revisions(id)?
            .into_iter()
            .find(|n| n.revision == revision)
            .ok_or(Error::NotFound)?;
        match self.restore_document(
            &Uuid::new_v4().to_string(),
            RestoreNote {
                note_id: id.into(),
                base_revision_id: current.revision_id,
                target_revision_id: target.revision_id,
            },
        ) {
            Err(Error::RevisionConflict { .. }) => Err(Error::Conflict {
                expected: base_revision,
                actual: self.note(id)?.revision,
            }),
            result => result.map(|result| result.note),
        }
    }

    pub fn list_notes(&self) -> Result<Vec<Note>> {
        let mut statement = self
            .connection
            .prepare("SELECT n.id FROM notes n JOIN objects o ON o.id=n.id WHERE n.trashed=0 AND o.archived=0 ORDER BY n.updated_at DESC,n.id")?;
        let ids = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter().map(|id| self.note(id)).collect()
    }

    pub fn set_trashed(&mut self, id: &str, trashed: bool) -> Result<()> {
        let base_row_version = self.object_metadata(id)?.row_version;
        self.set_object_trashed(
            &Uuid::new_v4().to_string(),
            SetObjectFlag {
                object_id: id.into(),
                base_row_version,
                value: trashed,
            },
        )?;
        Ok(())
    }

    pub fn create_collection(&mut self, name: &str) -> Result<Collection> {
        let result = self.collection_create(
            &Uuid::new_v4().to_string(),
            CreateCollection {
                name: name.into(),
                overview_note_id: None,
                is_pinned: false,
                pin_position: None,
            },
        )?;
        Ok(Collection {
            id: result.collection.id,
            name: result.collection.name,
        })
    }

    pub fn add_to_collection(&mut self, collection: &str, note: &str) -> Result<()> {
        let base_row_version = self.collection_get(collection)?.metadata.row_version;
        self.collection_add_members(
            &Uuid::new_v4().to_string(),
            ChangeCollectionMembers {
                collection_id: collection.into(),
                base_row_version,
                member_ids: vec![note.into()],
            },
        )?;
        Ok(())
    }

    pub fn remove_from_collection(&mut self, collection: &str, note: &str) -> Result<()> {
        let base_row_version = self.collection_get(collection)?.metadata.row_version;
        self.collection_remove_members(
            &Uuid::new_v4().to_string(),
            ChangeCollectionMembers {
                collection_id: collection.into(),
                base_row_version,
                member_ids: vec![note.into()],
            },
        )?;
        Ok(())
    }

    pub fn collection_notes(&self, collection: &str) -> Result<Vec<Note>> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE id=?1)",
            [collection],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        let mut statement = self.connection.prepare("SELECT n.id FROM notes n JOIN memberships m ON m.member_id=n.id WHERE m.collection_id=?1 AND n.trashed=0 ORDER BY m.position")?;
        let ids = statement
            .query_map([collection], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter().map(|id| self.note(id)).collect()
    }

    pub fn link_notes(&self, source: &str, target: &str, context: &str) -> Result<()> {
        if context.len() > 8192 {
            return Err(Error::InvalidInput("link context is too long"));
        }
        self.connection.execute("INSERT INTO note_links(source_id,target_id,context) VALUES(?1,?2,?3) ON CONFLICT(source_id,target_id) DO UPDATE SET context=excluded.context",params![source,target,context])?;
        Ok(())
    }

    pub fn backlinks(&self, target: &str) -> Result<Vec<NoteLink>> {
        self.note(target)?;
        let mut statement = self.connection.prepare("SELECT l.source_id,l.target_id,l.context FROM note_links l JOIN notes n ON n.id=l.source_id WHERE l.target_id=?1 AND n.trashed=0 ORDER BY l.source_id")?;
        Ok(statement
            .query_map([target], |r| {
                Ok(NoteLink {
                    source_id: r.get(0)?,
                    target_id: r.get(1)?,
                    context: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>> {
        if query.len() > 4096 || !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput("query or limit exceeds bounds"));
        }
        let terms = tokens(query);
        if terms.is_empty() {
            return Ok(vec![]);
        }
        // User text is always literal terms, never executable FTS query syntax.
        let query = terms
            .into_iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        let mut statement = self.connection.prepare("SELECT f.note_id,bm25(note_fts,0.0,3.0,1.0) AS score FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=n.id WHERE note_fts MATCH ?1 AND n.trashed=0 AND o.search_enabled=1 ORDER BY score,f.note_id LIMIT ?2")?;
        let rows = statement
            .query_map(params![query, limit], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(id, bm25)| {
                Ok(SearchHit {
                    note: self.note(&id)?,
                    bm25,
                })
            })
            .collect()
    }

    /// Database-only snapshot; includes workspace identity and migration history.
    /// Publication is atomic and never overwrites an existing destination.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<()> {
        snapshot::backup_to(&self.connection, destination.as_ref())
    }
}

fn validate_text(title: &str, body: &str) -> Result<()> {
    if title.trim().is_empty() || title.chars().count() > 300 {
        return Err(Error::InvalidInput("title must contain 1-300 characters"));
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(Error::InvalidInput("body exceeds 4 MiB"));
    }
    Ok(())
}

/// Shared tokenizer. A user term dictionary (`PKMS_SEARCH_TERMS`, otherwise
/// `%LOCALAPPDATA%\PKMS\search-terms.txt`) is folded in before the first use so
/// proper nouns stay single search terms.
pub(crate) fn tokenizer() -> &'static Jieba {
    TOKENIZER.get_or_init(|| {
        let mut jieba = Jieba::new();
        if let Some(path) = crate::chinese::dictionary_path() {
            for word in crate::chinese::dictionary_words(&path) {
                jieba.add_word(&word, None, None);
            }
        }
        jieba
    })
}

// Query side stays lexical: pinyin tokens are only added to documents, so a
// Chinese query never demands a pinyin token the document did not emit.
fn tokens(text: &str) -> Vec<String> {
    let folded = crate::chinese::fold(text);
    tokenizer()
        .cut_for_search(&folded, true)
        .into_iter()
        .filter(|t| t.word.chars().any(char::is_alphanumeric))
        .map(|t| t.word.to_lowercase())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn tokenize(text: &str) -> String {
    // Preserve term frequency and order for document BM25; only query terms are deduplicated.
    let folded = crate::chinese::fold(text);
    let mut words = tokenizer()
        .cut_for_search(&folded, true)
        .into_iter()
        .filter(|t| t.word.chars().any(char::is_alphanumeric))
        .map(|t| t.word.to_lowercase())
        .collect::<Vec<_>>();
    let pinyin = words
        .iter()
        .filter_map(|word| crate::chinese::pinyin_token(word))
        .collect::<Vec<_>>();
    words.extend(pinyin);
    words.join(" ")
}

fn index_note(connection: &Connection, id: &str, title: &str, body: &str) -> Result<()> {
    connection.execute("DELETE FROM note_fts WHERE note_id=?1", [id])?;
    insert_note_index(connection, id, title, body)
}

// The caller must guarantee this ID has no existing FTS row (a fresh note, or a
// full rebuild after clearing the index). FTS5 UNINDEXED IDs otherwise require
// scanning every stored document even when there is nothing to remove.
fn insert_note_index(connection: &Connection, id: &str, title: &str, body: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO note_fts(note_id,title_tokens,body_tokens) VALUES(?1,?2,?3)",
        params![id, title, body],
    )?;
    Ok(())
}
pub use handoff::{
    HandoffImport, HandoffObject, HandoffReadingPosition, RESUME_HANDOFF_FORMAT, ResumeHandoff,
};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// v1 intentionally uses ordinary, uncompressed ZIP entries. Limits are checked
/// before allocating the central directory or extracting any database bytes.
pub const BACKUP_FORMAT: u32 = 1;
pub const BACKUP_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const BACKUP_MAX_ENTRIES: usize = 20_000;
pub const BACKUP_MAX_DATABASE_BYTES: u64 = 1024 * 1024 * 1024;
pub const BACKUP_MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct BackupFile {
    pub path: String,
    pub sha256: String,
    #[ts(type = "number")]
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct BackupCounts {
    #[serde(default)]
    pub review_entries: u32,
    #[serde(default)]
    pub review_events: u32,
    #[serde(default)]
    pub index_rebuild_jobs: u32,
    #[serde(default)]
    pub embedding_profiles: u32,
    #[serde(default)]
    pub index_generations: u32,
    #[serde(default)]
    pub index_chunks: u32,
    #[serde(default)]
    pub answers: u32,
    #[serde(default)]
    pub answer_feedback: u32,
    #[serde(default)]
    pub proposals: u32,
    #[serde(default)]
    pub web_captures: u32,
    #[serde(default)]
    pub sources: u32,
    #[serde(default)]
    pub source_versions: u32,
    #[serde(default)]
    pub annotations: u32,
    #[serde(default)]
    pub source_jobs: u32,
    #[serde(default)]
    pub source_covers: u32,
    #[serde(default)]
    pub source_assets: u32,
    #[serde(default)]
    pub source_block_images: u32,
    #[serde(default)]
    pub voice_captures: u32,
    #[serde(default)]
    pub portable_jobs: u32,
    pub notes: u32,
    pub revisions: u32,
    pub objects: u32,
    pub collections: u32,
    pub memberships: u32,
    pub tags: u32,
    pub tag_assignments: u32,
    pub saved_views: u32,
    #[serde(default)]
    pub canvases: u32,
    pub manual_links: u32,
    pub body_links: u32,
    pub revision_blob_references: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub format_version: u32,
    pub workspace_id: String,
    pub app_version: String,
    pub database_schema: u32,
    pub document_schemas: Vec<u32>,
    pub locator_schemas: Vec<u32>,
    pub created_at: String,
    pub database: BackupFile,
    pub blobs: Vec<BackupFile>,
    pub counts: BackupCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct BackupInspection {
    pub manifest: BackupManifest,
    /// Container/manifest preflight only; this never means full verification.
    pub fully_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct BackupChatCounts {
    pub conversations: u32,
    pub messages: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct BackupReport {
    pub manifest: BackupManifest,
    pub verified_revisions: u32,
    pub verified_blobs: u32,
    /// Counts from the verified snapshot, not an extension of the frozen manifest.
    /// None means a persisted report predates this measurement, not zero chats.
    #[serde(default)]
    pub chat: Option<BackupChatCounts>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Markdown,
    Html,
    Docx,
    Pdf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct NoteExportReport {
    pub note_id: String,
    pub revision_id: String,
    pub format: ExportFormat,
    pub document_path: String,
    pub attachment_count: u32,
    pub warnings: Vec<String>,
}

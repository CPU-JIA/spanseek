//! The single source of truth for the stable `error_code` the desktop UI stores
//! and renders.
//!
//! Four entry points used to carry their own tables, and those tables had
//! drifted apart: a corrupt or unrelated library surfaced as `operation_failed`
//! when it was opened from the workspace entry point, while the very same
//! `rusqlite` failure was `integrity_failed` on the IPC path. Interface text has
//! to describe what the kernel really did, so every entry point derives its code
//! here instead of keeping a private copy.
//!
//! Only two things genuinely differ between entry points, and both stay explicit
//! rather than accidental:
//!
//! * the generic fallback code, because the front end keys its wording off
//!   `operation_failed` / `job_failed` / `inference_failed` /
//!   `source_import_failed`;
//! * whether a validation *reason* is itself a stable code. Inference and source
//!   import pass codes such as `embedding_remote_required` or `pdf_invalid` that
//!   the UI renders, while workspace operations pass diagnostic sentences that
//!   must not leak into a stored code.

use super::AppError;

/// The entry point that observed a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorDomain {
    /// `app_operations`: opening, creating, verifying and restoring a workspace.
    Workspace,
    /// `portable_jobs`: export, backup and diagnostics tasks.
    PortableJob,
    /// Search, answers, chat and generation jobs.
    Inference,
    /// Source import and parsing jobs.
    SourceImport,
}

impl ErrorDomain {
    /// The generic code this entry point reports when nothing more specific
    /// applies. The front end renders these, so they are part of the contract.
    fn fallback(self) -> &'static str {
        match self {
            Self::Workspace => "operation_failed",
            Self::PortableJob => "job_failed",
            Self::Inference => "inference_failed",
            Self::SourceImport => "source_import_failed",
        }
    }
}

/// Derives the stable code for `error` as reported by `domain`.
///
/// Every classification below is shared, so the same kernel failure can never
/// read differently depending on which entry point observed it.
pub(crate) fn error_code(error: &AppError, domain: ErrorDomain) -> &'static str {
    match error {
        // A corrupt, busy or full database is classified exactly like the IPC
        // path's `database_code`; anything else keeps the domain's own code.
        AppError::Database(error) | AppError::Core(pkms_core::Error::Database(error)) => {
            database_code(error).unwrap_or_else(|| domain.fallback())
        }
        AppError::InputUnavailable => "input_unavailable",
        AppError::OutputExists => "output_exists",
        AppError::InvalidInput(reason) => invalid_input_code(reason, domain),
        AppError::Core(pkms_core::Error::InvalidInput(reason)) => {
            core_invalid_input_code(reason, domain)
        }
        AppError::Core(pkms_core::Error::InvalidDocument(_)) => "invalid_document",
        AppError::AppBusy | AppError::Core(pkms_core::Error::WorkspaceBusy) => "storage_busy",
        AppError::UnsupportedDatabase | AppError::Core(pkms_core::Error::UnsupportedDatabase) => {
            "unsupported_database"
        }
        AppError::Core(pkms_core::Error::MigrationHistoryMismatch { .. }) => "migration_failed",
        AppError::Core(pkms_core::Error::IntegrityFailure) => "integrity_failed",
        // A source import whose workspace handle went away is reported as an
        // unavailable input, which is what the import surface can act on.
        AppError::InstanceMismatch if domain == ErrorDomain::SourceImport => "input_unavailable",
        AppError::Io(error) | AppError::Core(pkms_core::Error::Io(error)) => io_code(error),
        _ => domain.fallback(),
    }
}

/// `Interrupted` is this code base's cancellation signal (a canceled job or a
/// worker shutting down), so it must not surface as an I/O failure.
fn io_code(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::Interrupted => "canceled",
        std::io::ErrorKind::StorageFull => "storage_full",
        _ => "io_failed",
    }
}

/// Workspace operations carry diagnostic sentences in `InvalidInput`, while
/// inference and source import carry stable codes the UI renders.
fn invalid_input_code(reason: &'static str, domain: ErrorDomain) -> &'static str {
    match domain {
        ErrorDomain::Workspace => "invalid_input",
        ErrorDomain::Inference | ErrorDomain::SourceImport => reason,
        ErrorDomain::PortableJob if is_portable_export_code(reason) => reason,
        ErrorDomain::PortableJob => domain.fallback(),
    }
}

fn core_invalid_input_code(reason: &'static str, domain: ErrorDomain) -> &'static str {
    match domain {
        ErrorDomain::Workspace => "invalid_input",
        // Agent tools report their own stable reason codes.
        ErrorDomain::Inference if reason.starts_with("agent_") => reason,
        // Anything else that stops a source import is an unusable source.
        ErrorDomain::SourceImport => "invalid_source",
        ErrorDomain::PortableJob if is_portable_export_code(reason) => reason,
        ErrorDomain::Inference | ErrorDomain::PortableJob => domain.fallback(),
    }
}

fn is_portable_export_code(reason: &str) -> bool {
    matches!(
        reason,
        "pdf_renderer_unavailable" | "pdf_render_failed" | "pdf_render_timeout"
    )
}

/// Mirrors the IPC path's `database_code` classification: a corrupt or unrelated
/// file is an integrity failure, a full disk a storage failure, and a locked or
/// busy file a busy workspace. `None` leaves the code to the entry point.
fn database_code(error: &rusqlite::Error) -> Option<&'static str> {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DiskFull) => Some("storage_full"),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            Some("storage_busy")
        }
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            Some("integrity_failed")
        }
        _ => None,
    }
}

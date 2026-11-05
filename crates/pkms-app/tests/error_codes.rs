//! One kernel failure must read the same from every entry point.
//!
//! `error_codes` is a `pub(crate)` module, so this test includes the source file
//! directly instead of widening the crate's public API just for testing.

use pkms_app::AppError;

#[path = "../src/error_codes.rs"]
mod error_codes;

use error_codes::ErrorDomain;

/// Every entry point that stores or renders an `error_code`.
const DOMAINS: [ErrorDomain; 4] = [
    ErrorDomain::Workspace,
    ErrorDomain::PortableJob,
    ErrorDomain::Inference,
    ErrorDomain::SourceImport,
];

fn assert_code(error: &AppError, expected: &str, label: &str) {
    for domain in DOMAINS {
        assert_eq!(
            error_codes::error_code(error, domain),
            expected,
            "{label} reported from {domain:?}"
        );
    }
}

/// The failure SQLite reports for a file that is not one of our databases.
fn not_a_database() -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_NOTADB),
        Some("file is not a database".into()),
    )
}

/// A corrupt or unrelated library is an integrity failure everywhere; it used to
/// read as `operation_failed` from the workspace entry point and as
/// `integrity_failed` on the IPC path.
#[test]
fn corrupt_library_reads_the_same_from_every_entry_point() {
    let core = AppError::Core(pkms_core::Error::Database(not_a_database()));
    assert_code(&core, "integrity_failed", "core database failure");

    let app = AppError::Database(not_a_database());
    assert_code(&app, "integrity_failed", "application database failure");
}

/// A file with the wrong header is the same failure the user actually hits.
#[test]
fn a_file_that_is_not_a_database_is_an_integrity_failure() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("library.sqlite3");
    std::fs::write(&path, b"spanseek is not a SQLite database").unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let error = connection
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_err();
    assert!(
        matches!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt)
        ),
        "unexpected sqlite failure for a non-database file: {error:?}"
    );
    assert_code(
        &AppError::Core(pkms_core::Error::Database(error)),
        "integrity_failed",
        "non-database file",
    );
}

/// Each of the five user-visible database classes keeps one stable code.
#[test]
fn database_classes_keep_one_code_per_class() {
    let cases: [(AppError, &str); 5] = [
        (
            AppError::Core(pkms_core::Error::Database(not_a_database())),
            "integrity_failed",
        ),
        (
            AppError::Core(pkms_core::Error::UnsupportedDatabase),
            "unsupported_database",
        ),
        (
            AppError::Core(pkms_core::Error::MigrationHistoryMismatch { version: 18 }),
            "migration_failed",
        ),
        (
            AppError::Core(pkms_core::Error::WorkspaceBusy),
            "storage_busy",
        ),
        (
            AppError::Io(std::io::Error::from(std::io::ErrorKind::StorageFull)),
            "storage_full",
        ),
    ];
    for (error, expected) in &cases {
        assert_code(error, expected, "database class");
    }
}

/// The remaining shared classifications, including a busy database and a full
/// disk reported through SQLite itself.
#[test]
fn shared_classifications_agree_across_entry_points() {
    let cases: [(AppError, &str); 6] = [
        (
            AppError::Core(pkms_core::Error::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                None,
            ))),
            "storage_busy",
        ),
        (
            AppError::Core(pkms_core::Error::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
                None,
            ))),
            "storage_full",
        ),
        (AppError::AppBusy, "storage_busy"),
        (AppError::UnsupportedDatabase, "unsupported_database"),
        (
            AppError::Core(pkms_core::Error::IntegrityFailure),
            "integrity_failed",
        ),
        (
            AppError::Core(pkms_core::Error::InvalidDocument("unknown node")),
            "invalid_document",
        ),
    ];
    for (error, expected) in &cases {
        assert_code(error, expected, "shared classification");
    }
}

/// Cancellation and I/O failures keep their established codes.
#[test]
fn io_classifications_agree_across_entry_points() {
    let cases: [(AppError, &str); 3] = [
        (
            AppError::Core(pkms_core::Error::Io(std::io::Error::from(
                std::io::ErrorKind::Interrupted,
            ))),
            "canceled",
        ),
        (
            AppError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            "io_failed",
        ),
        (
            AppError::Core(pkms_core::Error::Io(std::io::Error::from(
                std::io::ErrorKind::StorageFull,
            ))),
            "storage_full",
        ),
    ];
    for (error, expected) in &cases {
        assert_code(error, expected, "io classification");
    }
}

/// The per-domain differences are deliberate and stay unchanged: each entry
/// point keeps its own generic code, and only the surfaces that render stable
/// reasons may surface a reason string.
#[test]
fn domain_specific_codes_are_preserved() {
    assert_eq!(
        error_codes::error_code(&AppError::StateConflict, ErrorDomain::Workspace),
        "operation_failed"
    );
    assert_eq!(
        error_codes::error_code(&AppError::StateConflict, ErrorDomain::PortableJob),
        "job_failed"
    );
    assert_eq!(
        error_codes::error_code(&AppError::StateConflict, ErrorDomain::Inference),
        "inference_failed"
    );
    assert_eq!(
        error_codes::error_code(&AppError::StateConflict, ErrorDomain::SourceImport),
        "source_import_failed"
    );

    // Workspace operations pass diagnostic sentences; they must not be stored.
    assert_eq!(
        error_codes::error_code(
            &AppError::InvalidInput("identity must be a UUID"),
            ErrorDomain::Workspace
        ),
        "invalid_input"
    );
    // Inference and source import pass stable codes the interface renders.
    assert_eq!(
        error_codes::error_code(
            &AppError::InvalidInput("pdf_invalid"),
            ErrorDomain::SourceImport
        ),
        "pdf_invalid"
    );
    assert_eq!(
        error_codes::error_code(
            &AppError::Core(pkms_core::Error::InvalidInput("agent_tool_failed")),
            ErrorDomain::Inference
        ),
        "agent_tool_failed"
    );
    for code in [
        "pdf_renderer_unavailable",
        "pdf_render_failed",
        "pdf_render_timeout",
    ] {
        assert_eq!(
            error_codes::error_code(
                &AppError::Core(pkms_core::Error::InvalidInput(code)),
                ErrorDomain::PortableJob
            ),
            code
        );
    }
    assert_eq!(
        error_codes::error_code(
            &AppError::Core(pkms_core::Error::InvalidInput("unsupported type")),
            ErrorDomain::SourceImport
        ),
        "invalid_source"
    );
    assert_eq!(
        error_codes::error_code(
            &AppError::InvalidInput("worker_failed"),
            ErrorDomain::SourceImport
        ),
        "worker_failed"
    );
    // A source import whose workspace handle vanished is an unavailable input.
    assert_eq!(
        error_codes::error_code(&AppError::InstanceMismatch, ErrorDomain::SourceImport),
        "input_unavailable"
    );
    assert_eq!(
        error_codes::error_code(&AppError::InstanceMismatch, ErrorDomain::Workspace),
        "operation_failed"
    );
    assert_eq!(
        error_codes::error_code(&AppError::OutputExists, ErrorDomain::Workspace),
        "output_exists"
    );
}

//! Top-K / citation-count / similarity-floor settings: defaults, versioning,
//! idempotency, validation and the K17 -> K18 upgrade path.
mod common;

use pkms_core::*;
use rusqlite::Connection;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

#[test]
fn retrieval_settings_default_roundtrip_replay_and_conflict() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let initial = db.retrieval_settings().unwrap();
    assert_eq!(initial.params, RetrievalParams::default());
    assert_eq!(initial.row_version, 0);
    assert_eq!(initial.params.top_k, 8);
    assert_eq!(initial.params.citation_count, 5);
    assert_eq!(initial.params.minimum_cosine, None);

    let command = id();
    let request = SetRetrievalParams {
        base_row_version: 0,
        params: RetrievalParams {
            top_k: 12,
            citation_count: 4,
            minimum_cosine: Some(0.32),
            semantic_enabled: true,
            remote_consent: false,
            ..Default::default()
        },
    };
    let saved = db.set_retrieval_params(&command, request.clone()).unwrap();
    assert_eq!(saved.settings.row_version, 1);
    assert!(!saved.receipt.replayed);
    assert_eq!(db.retrieval_settings().unwrap(), saved.settings);
    let replayed = db.set_retrieval_params(&command, request).unwrap();
    assert!(replayed.receipt.replayed);
    assert_eq!(replayed.settings, saved.settings);
    assert!(matches!(
        db.set_retrieval_params(
            &id(),
            SetRetrievalParams {
                base_row_version: 0,
                params: RetrievalParams::default(),
            }
        ),
        Err(Error::MetadataConflict { .. })
    ));
    assert_eq!(db.retrieval_settings().unwrap(), saved.settings);
    assert!(matches!(
        db.set_retrieval_params(
            &command,
            SetRetrievalParams {
                base_row_version: 1,
                params: RetrievalParams::default(),
            }
        ),
        Err(Error::IdempotencyMismatch)
    ));
    drop(db);
    let reopened = Library::open_existing(root.path()).unwrap();
    assert_eq!(reopened.retrieval_settings().unwrap(), saved.settings);
}

#[test]
fn retrieval_settings_reject_out_of_range_without_a_write() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    for params in [
        RetrievalParams {
            top_k: 0,
            ..Default::default()
        },
        RetrievalParams {
            top_k: 51,
            ..Default::default()
        },
        RetrievalParams {
            citation_count: 0,
            ..Default::default()
        },
        RetrievalParams {
            citation_count: 21,
            ..Default::default()
        },
        RetrievalParams {
            top_k: 3,
            citation_count: 4,
            minimum_cosine: None,
            semantic_enabled: true,
            remote_consent: false,
            ..Default::default()
        },
        RetrievalParams {
            minimum_cosine: Some(1.5),
            ..Default::default()
        },
        RetrievalParams {
            minimum_cosine: Some(f64::NAN),
            ..Default::default()
        },
        RetrievalParams {
            answer_timeout_seconds: Some(MIN_ANSWER_TIMEOUT_SECONDS - 1),
            ..Default::default()
        },
        RetrievalParams {
            answer_timeout_seconds: Some(MAX_ANSWER_TIMEOUT_SECONDS + 1),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            db.set_retrieval_params(
                &id(),
                SetRetrievalParams {
                    base_row_version: 0,
                    params,
                }
            ),
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(db.retrieval_settings().unwrap().row_version, 0);
    }
    let saved = db
        .set_retrieval_params(
            &id(),
            SetRetrievalParams {
                base_row_version: 0,
                params: RetrievalParams {
                    top_k: 50,
                    citation_count: 20,
                    minimum_cosine: Some(-1.0),
                    semantic_enabled: true,
                    remote_consent: false,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    assert_eq!(saved.settings.params.top_k, 50);
    assert_eq!(saved.settings.params.citation_count, 20);
    assert_eq!(saved.settings.params.minimum_cosine, Some(-1.0));
}

#[test]
fn an_absent_answer_budget_stays_absent_and_a_present_one_round_trips() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    // Absent means "let the provider decide"; the stored row must not invent 0.
    assert_eq!(
        db.retrieval_settings()
            .unwrap()
            .params
            .answer_timeout_seconds,
        None
    );
    for value in [MIN_ANSWER_TIMEOUT_SECONDS, 180, MAX_ANSWER_TIMEOUT_SECONDS] {
        let saved = db
            .set_retrieval_params(
                &id(),
                SetRetrievalParams {
                    base_row_version: db.retrieval_settings().unwrap().row_version,
                    params: RetrievalParams {
                        answer_timeout_seconds: Some(value),
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        assert_eq!(saved.settings.params.answer_timeout_seconds, Some(value));
    }
    let cleared = db
        .set_retrieval_params(
            &id(),
            SetRetrievalParams {
                base_row_version: db.retrieval_settings().unwrap().row_version,
                params: RetrievalParams::default(),
            },
        )
        .unwrap();
    assert_eq!(cleared.settings.params.answer_timeout_seconds, None);
    drop(db);
    let reopened = Library::open_existing(root.path()).unwrap();
    assert_eq!(
        reopened
            .retrieval_settings()
            .unwrap()
            .params
            .answer_timeout_seconds,
        None
    );
}

#[test]
fn k17_upgrade_adds_retrieval_params_and_rejects_a_corrupted_snapshot() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("迁移原文", "检索参数迁移").unwrap();
    drop(db);
    let path = root.path().join("library.sqlite3");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(&format!("{versions} ALTER TABLE search_settings DROP COLUMN retrieval_params_json; ALTER TABLE search_settings DROP COLUMN retrieval_row_version; DELETE FROM migration_history WHERE version=18; DROP TABLE retrieval_events; DROP TABLE model_usage_events; DELETE FROM migration_history WHERE version=19; DELETE FROM migration_history WHERE version=20; ALTER TABLE sources DROP COLUMN author; DELETE FROM migration_history WHERE version=21; ALTER TABLE search_settings DROP COLUMN cleaning_params_json; ALTER TABLE search_settings DROP COLUMN cleaning_row_version; ALTER TABLE source_parses DROP COLUMN cleaning_json; DELETE FROM migration_history WHERE version=22; ALTER TABLE search_settings DROP COLUMN chunk_params_json; ALTER TABLE search_settings DROP COLUMN chunk_row_version; ALTER TABLE search_settings DROP COLUMN previous_profile_id; DELETE FROM migration_history WHERE version=23; DELETE FROM migration_history WHERE version=24; {views} {automation} UPDATE workspace_meta SET schema_version=17; PRAGMA user_version=17;", versions = common::pre_docx_source_versions(), views = common::pre_views_canvas_versions(), automation = common::pre_automation_versions())).unwrap();
    drop(raw);
    let mut upgraded = Library::open_existing(root.path()).unwrap();
    assert_eq!(
        upgraded.info().unwrap().database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    assert_eq!(upgraded.note(&note.id).unwrap().document, note.document);
    assert_eq!(
        upgraded.retrieval_settings().unwrap().params,
        RetrievalParams::default()
    );
    upgraded
        .set_retrieval_params(
            &id(),
            SetRetrievalParams {
                base_row_version: 0,
                params: RetrievalParams {
                    top_k: 6,
                    citation_count: 3,
                    minimum_cosine: None,
                    semantic_enabled: true,
                    remote_consent: false,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    let package = root.path().join("retrieval.pkmsbackup");
    upgraded.create_backup(&package, &mut |_| Ok(())).unwrap();
    verify_backup(&package, &mut |_| Ok(())).unwrap();
    drop(upgraded);
    // An in-range snapshot still packages normally, so the rejection below is
    // caused by the value check rather than by any unrelated integrity failure.
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "UPDATE search_settings SET retrieval_params_json=?1",
        ["{\"top_k\":9,\"citation_count\":4}"],
    )
    .unwrap();
    drop(raw);
    let accepted = root.path().join("accepted.pkmsbackup");
    Library::open_existing(root.path())
        .unwrap()
        .create_backup(&accepted, &mut |_| Ok(()))
        .unwrap();
    verify_backup(&accepted, &mut |_| Ok(())).unwrap();
    // Out-of-range values are rejected instead of being packaged silently.
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "UPDATE search_settings SET retrieval_params_json=?1",
        ["{\"top_k\":0,\"citation_count\":5}"],
    )
    .unwrap();
    drop(raw);
    let broken = root.path().join("broken.pkmsbackup");
    assert!(matches!(
        Library::open_existing(root.path())
            .unwrap()
            .create_backup(&broken, &mut |_| Ok(())),
        Err(Error::IntegrityFailure)
    ));
}

#[test]
fn legacy_retrieval_params_json_loads_with_semantics_enabled() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    // A row written before semantic_enabled/remote_consent existed.
    raw.execute(
        "UPDATE search_settings SET retrieval_params_json=?1,retrieval_row_version=1",
        [r#"{"top_k":12,"citation_count":4,"minimum_cosine":0.25}"#],
    )
    .unwrap();
    drop(raw);
    let settings = db.retrieval_settings().unwrap();
    assert_eq!(settings.params.top_k, 12);
    assert_eq!(settings.params.minimum_cosine, Some(0.25));
    assert!(settings.params.semantic_enabled);
    assert!(!settings.params.remote_consent);
    let saved = db
        .set_retrieval_params(
            &id(),
            SetRetrievalParams {
                base_row_version: 1,
                params: settings.params,
            },
        )
        .unwrap();
    assert_eq!(saved.settings.row_version, 2);
}
#[test]
fn reference_prices_roundtrip_and_reject_unusable_values() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    assert_eq!(
        db.retrieval_settings()
            .unwrap()
            .params
            .cost_per_million_input,
        None
    );
    let saved = db
        .set_retrieval_params(
            &id(),
            SetRetrievalParams {
                base_row_version: 0,
                params: RetrievalParams {
                    cost_per_million_input: Some(0.5),
                    cost_per_million_output: Some(2.0),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    assert_eq!(saved.settings.params.cost_per_million_input, Some(0.5));
    assert_eq!(saved.settings.params.cost_per_million_output, Some(2.0));
    assert_eq!(
        db.retrieval_settings().unwrap().params,
        saved.settings.params
    );
    for params in [
        RetrievalParams {
            cost_per_million_input: Some(-0.01),
            ..Default::default()
        },
        RetrievalParams {
            cost_per_million_output: Some(f64::INFINITY),
            ..Default::default()
        },
        RetrievalParams {
            cost_per_million_input: Some(MAX_COST_PER_MILLION + 1.0),
            ..Default::default()
        },
        RetrievalParams {
            cost_per_million_output: Some(f64::NAN),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            db.set_retrieval_params(
                &id(),
                SetRetrievalParams {
                    base_row_version: 1,
                    params,
                }
            ),
            Err(Error::InvalidInput(_))
        ));
    }
    // Refused writes leave the stored row exactly as it was.
    assert_eq!(db.retrieval_settings().unwrap(), saved.settings);
    // A stored price outside the accepted range fails backup validation instead
    // of being packaged silently.
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE search_settings SET retrieval_params_json=?1",
        [r#"{"top_k":8,"citation_count":5,"cost_per_million_input":-3}"#],
    )
    .unwrap();
    drop(raw);
    let broken = root.path().join("prices.pkmsbackup");
    assert!(matches!(
        Library::open_existing(root.path())
            .unwrap()
            .create_backup(&broken, &mut |_| Ok(())),
        Err(Error::IntegrityFailure)
    ));
}

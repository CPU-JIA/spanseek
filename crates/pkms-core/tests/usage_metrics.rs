//! Local metrics for the statistics panel: rolling windows, honest unknown
//! handling and pruning.
use pkms_core::*;
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn retrieval_latency_window_reports_empty_then_real_samples() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let empty = db.workspace_statistics().unwrap().retrieval_latency_24h;
    assert_eq!(empty.samples, 0);
    assert_eq!(empty.average_ms, 0);
    assert_eq!(empty.p95_ms, 0);
    for (mode, duration) in [
        (SearchMode::Lexical, 10_u64),
        (SearchMode::Hybrid, 20),
        (SearchMode::Semantic, 30),
        (SearchMode::Hybrid, 40),
    ] {
        db.record_retrieval(mode, duration, 3).unwrap();
    }
    let stats = db.workspace_statistics().unwrap().retrieval_latency_24h;
    assert_eq!(stats.samples, 4);
    assert_eq!(stats.average_ms, 25);
    // Nearest rank at index floor(samples * 0.95) on the sorted sample.
    assert_eq!(stats.p95_ms, 40);
    assert!(matches!(
        db.record_retrieval(SearchMode::Lexical, 3_600_001, 0),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(
        db.workspace_statistics()
            .unwrap()
            .retrieval_latency_24h
            .samples,
        4
    );
}

#[test]
fn model_usage_totals_separate_estimates_and_unknowns() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let empty = db.workspace_statistics().unwrap().model_usage_30d;
    assert_eq!(empty.calls, 0);
    assert_eq!(empty.estimated_calls, 0);
    assert_eq!(empty.calls_without_usage, 0);
    assert_eq!(empty.timed_calls, 0);
    assert_eq!(empty.average_duration_ms, None);
    assert_eq!(empty.average_first_token_ms, None);
    db.record_model_usage(
        UsagePurpose::Answer,
        "openai_chat",
        "fixture-model",
        ModelUsage {
            input_tokens: Some(100),
            output_tokens: Some(50),
            estimated: false,
            latency: Some(ModelCallLatency {
                duration_ms: 1_200,
                first_token_ms: Some(300),
            }),
        },
    )
    .unwrap();
    db.record_model_usage(
        UsagePurpose::Chat,
        "openai_chat",
        "fixture-model",
        ModelUsage {
            input_tokens: None,
            output_tokens: None,
            estimated: false,
            latency: Some(ModelCallLatency {
                duration_ms: 900,
                first_token_ms: Some(450),
            }),
        },
    )
    .unwrap();
    db.record_model_usage(
        UsagePurpose::Embedding,
        "openai_embeddings",
        "@cf/baai/bge-m3",
        ModelUsage {
            input_tokens: Some(80),
            output_tokens: None,
            estimated: true,
            latency: None,
        },
    )
    .unwrap();
    let stats = db.workspace_statistics().unwrap().model_usage_30d;
    assert_eq!(stats.calls, 3);
    assert_eq!(stats.estimated_calls, 1);
    assert_eq!(stats.calls_without_usage, 1);
    assert_eq!(stats.input_tokens, 180);
    assert_eq!(stats.output_tokens, 50);
    // Latency is reported only for the calls that measured it; the embedding
    // call stays out of the average instead of counting as zero.
    assert_eq!(stats.timed_calls, 2);
    assert_eq!(stats.average_duration_ms, Some(1_050));
    assert_eq!(stats.average_first_token_ms, Some(375));
    assert!(matches!(
        db.record_model_usage(
            UsagePurpose::Answer,
            "",
            "fixture-model",
            ModelUsage {
                input_tokens: None,
                output_tokens: None,
                estimated: false,
                latency: None,
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    // A first token that arrives after the call finished is not a measurement.
    assert!(matches!(
        db.record_model_usage(
            UsagePurpose::Answer,
            "openai_chat",
            "fixture-model",
            ModelUsage {
                input_tokens: None,
                output_tokens: None,
                estimated: false,
                latency: Some(ModelCallLatency {
                    duration_ms: 10,
                    first_token_ms: Some(20),
                }),
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(db.workspace_statistics().unwrap().model_usage_30d.calls, 3);
}

#[test]
fn metrics_prune_beyond_retention_and_survive_reopen() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    db.record_retrieval(SearchMode::Hybrid, 12, 1).unwrap();
    db.record_model_usage(
        UsagePurpose::Answer,
        "openai_chat",
        "fixture-model",
        ModelUsage {
            input_tokens: Some(1),
            output_tokens: Some(2),
            estimated: false,
            latency: Some(ModelCallLatency {
                duration_ms: 5,
                first_token_ms: None,
            }),
        },
    )
    .unwrap();
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE retrieval_events SET occurred_at='2000-01-01T00:00:00+00:00'",
        [],
    )
    .unwrap();
    raw.execute(
        "UPDATE model_usage_events SET occurred_at='2000-01-01T00:00:00+00:00'",
        [],
    )
    .unwrap();
    drop(raw);
    db.record_retrieval(SearchMode::Lexical, 5, 1).unwrap();
    let stats = db.workspace_statistics().unwrap();
    assert_eq!(stats.retrieval_latency_24h.samples, 1);
    assert_eq!(stats.model_usage_30d.calls, 0);
    assert_eq!(stats.model_usage_30d.timed_calls, 0);
    assert_eq!(stats.model_usage_30d.average_duration_ms, None);
    drop(db);
    let reopened = Library::open_existing(root.path()).unwrap();
    assert_eq!(
        reopened
            .workspace_statistics()
            .unwrap()
            .retrieval_latency_24h
            .samples,
        1
    );
}

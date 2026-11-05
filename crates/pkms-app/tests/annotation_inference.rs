//! Opt-in real inference checks against isolated libraries, never user workspaces.
use pkms_app::{AppService, GrantPurpose, InferenceConfig, OperationKind, WorkspaceIdentity};
use pkms_core::*;
use std::path::{Path, PathBuf};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
const QUOTE: &str = "Marigold service keeps records for 45 days.";
const BEFORE: &str = "FORBIDDEN_OUTSIDE_BEFORE secret 991. ";
const AFTER: &str = " FORBIDDEN_OUTSIDE_AFTER internal password 882.";

fn import(db: &mut Library, text: &str, previous: Option<&SourceSummary>) -> SourceVersion {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "FORBIDDEN_SOURCE_TITLE".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: previous.map(|s| s.id.clone()),
                base_row_version: previous.map(|s| s.row_version),
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
    )
    .unwrap();
    version
}
fn setup(
    root: &Path,
) -> (
    AppService,
    WorkspaceIdentity,
    SourceVersion,
    Annotation,
    String,
) {
    let mut app = AppService::open(root.join("app")).unwrap();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, root.join("library"))
        .unwrap();
    let operation = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let identity = app
        .execute_operation(&operation.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity;
    let db = app.library_mut(&identity).unwrap();
    let source = import(db, &format!("{BEFORE}{QUOTE}{AFTER}"), None);
    let block = db
        .source_text_page(&source.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    let start = BEFORE.chars().count() as u32;
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: source.id.clone(),
                    block_id: block.id,
                    start_codepoint: start,
                    end_codepoint: start + QUOTE.chars().count() as u32,
                    quote: QUOTE.into(),
                    prefix: BEFORE.into(),
                    suffix: AFTER.into(),
                },
                comment: "FORBIDDEN_ANNOTATION_COMMENT".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let collection = db.create_collection("Quote-only topic").unwrap();
    db.collection_add_members(
        &id(),
        ChangeCollectionMembers {
            collection_id: collection.id.clone(),
            base_row_version: db
                .collection_get(&collection.id)
                .unwrap()
                .metadata
                .row_version,
            member_ids: vec![annotation.id.clone()],
        },
    )
    .unwrap();
    let previous = db.source(&source.source_id).unwrap();
    import(
        db,
        "The current parent version does not contain the authorized historical fact.",
        Some(&previous),
    );
    (app, identity, source, annotation, collection.id)
}
fn config(root: &Path) -> InferenceConfig {
    InferenceConfig {
        executable: PathBuf::from(
            std::env::var_os("PKMS_INFERENCE_EXE")
                .expect("explicit freshly built runtime required"),
        )
        .canonicalize()
        .unwrap(),
        model_directory: PathBuf::from(
            std::env::var_os("PKMS_EMBEDDING_DIR").expect("verified E5 directory required"),
        ),
        vector_root: root.join("vectors"),
        shutdown: Default::default(),
    }
}
fn query(collection: &str, mode: SearchMode) -> SearchQuery {
    SearchQuery {
        query: "How long does Marigold keep records?".into(),
        mode,
        scope: SearchScope {
            collection_id: Some(collection.into()),
            ..Default::default()
        },
        limit: 8,
        minimum_cosine: None,
    }
}
fn assert_quote(evidence: &SearchEvidence, source: &SourceVersion, annotation: &Annotation) {
    assert_eq!(
        evidence.annotation.as_ref().unwrap().annotation_id,
        annotation.id
    );
    assert_eq!(evidence.target.object_id, source.source_id);
    assert_eq!(evidence.target.content_revision, source.id);
    assert_eq!(evidence.title, "摘录");
    assert_eq!(evidence.text, QUOTE);
    assert_eq!(
        evidence.target.start_codepoint,
        BEFORE.chars().count() as u32
    );
    assert_eq!(
        evidence.target.end_codepoint,
        (BEFORE.chars().count() + QUOTE.chars().count()) as u32
    );
    assert!(evidence.generation_id.is_none());
}

#[test]
#[ignore = "requires freshly built runtime and verified E5 assets; no Ollama"]
fn real_e5_searches_only_the_historical_quote_without_a_whole_source_index() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity, source, annotation, collection) = setup(root.path());
    let profile = app
        .library_mut(&identity)
        .unwrap()
        .register_embedding_profile(&id(), pkms_inference::profile::profile_spec())
        .unwrap()
        .profile;
    assert!(
        app.library(&identity)
            .unwrap()
            .active_index_generation_ids()
            .unwrap()
            .is_empty()
    );
    let cfg = config(root.path());
    let mut connection = app.inference_connection(&identity).unwrap();
    for mode in [
        SearchMode::Lexical,
        SearchMode::Semantic,
        SearchMode::Hybrid,
    ] {
        let results = connection.search(&cfg, query(&collection, mode)).unwrap();
        assert!(
            !results.items.is_empty(),
            "{mode:?}: {:?}",
            results.unavailable_reason
        );
        for evidence in &results.items {
            assert_quote(evidence, &source, &annotation);
        }
        if mode != SearchMode::Lexical {
            assert!(results.semantic_available);
            assert_eq!(results.profile_id.as_deref(), Some(profile.id.as_str()));
            assert!(results.items[0].cosine_similarity.unwrap().is_finite());
        }
    }
    let hidden = app
        .library_mut(&identity)
        .unwrap()
        .create_collection("Empty topic")
        .unwrap();
    assert!(
        connection
            .search(&cfg, query(&hidden.id, SearchMode::Semantic))
            .unwrap()
            .items
            .is_empty()
    );
    app.library_mut(&identity)
        .unwrap()
        .set_annotation_trashed(
            &id(),
            SetObjectFlag {
                object_id: annotation.id,
                base_row_version: annotation.row_version,
                value: true,
            },
        )
        .unwrap();
    assert!(
        connection
            .search(&cfg, query(&collection, SearchMode::Hybrid))
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
#[ignore = "requires explicit isolated Ollama with admitted 4B, fresh runtime, and verified E5 path"]
fn real_ollama_answer_freezes_only_authorized_quote_and_persists_citation() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity, source, annotation, collection) = setup(root.path());
    let profile = GenerationProfile {
        channel_id: None,
        capabilities: None,
        chat_token_limit: None,
        provider: "ollama".into(),
        endpoint: std::env::var("PKMS_TEST_OLLAMA_ENDPOINT")
            .expect("explicit isolated endpoint required"),
        model: "qwen3.5:4b".into(),
        model_digest: "2a654d98e6fba55d452b7043684e9b57a947e393bbffa62485a7aac05ee4eefd".into(),
        config_revision: 1,
        temperature: 0.0,
        context_tokens: 8192,
    };
    pkms_app::verify_generation_model(&profile, &Default::default()).unwrap();
    let db = app.library_mut(&identity).unwrap();
    db.register_embedding_profile(&id(), pkms_inference::profile::profile_spec())
        .unwrap();
    let base = db.generation_settings().unwrap().row_version;
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: base,
            profile: Some(profile),
        },
    )
    .unwrap();
    let answer = db
        .accept_answer_from_settings(&id(), query(&collection, SearchMode::Hybrid))
        .unwrap()
        .answer;
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config(root.path()), &mut |_, _, _| {})
        .unwrap();
    let result = app.library(&identity).unwrap().answer(&answer.id).unwrap();
    assert_eq!(
        result.status,
        AnswerStatus::Completed,
        "{:?}",
        result.error_code
    );
    assert!(result.text.contains("45"));
    assert!(result.text.contains("[E1]"));
    assert!(!result.text.contains("FORBIDDEN"));
    assert_eq!(result.evidence.len(), 1);
    assert_quote(&result.evidence[0].evidence, &source, &annotation);
    assert!(result.evidence[0].evidence.cosine_similarity.is_some());
}

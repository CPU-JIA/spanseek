use super::*;
use pkms_core::{
    CreateNote, EmbeddingProfile, RebuildLease, RebuildRequest, SaveNote, SearchScope,
};
use serde_json::json;

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn fixture(
    count: usize,
    blocks: usize,
) -> (
    tempfile::TempDir,
    AppService,
    WorkspaceIdentity,
    Vec<String>,
) {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let grant = app
        .grant_path(
            crate::GrantPurpose::CreateWorkspace,
            root.path().join("knowledge"),
        )
        .unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), crate::OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let identity = app
        .execute_operation(&accepted.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity;
    let mut notes = Vec::new();
    for n in 0..count {
        let content = (0..blocks).map(|i| json!({"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":format!("Synthetic object {n} block {i}")}]})).collect::<Vec<_>>();
        let note = app
            .library_mut(&identity)
            .unwrap()
            .create_document(
                &id(),
                CreateNote {
                    title: format!("Synthetic {n}"),
                    document_schema: 2,
                    document: serde_json::from_value(json!({"type":"doc","content":content}))
                        .unwrap(),
                    collection_id: None,
                    base_collection_row_version: None,
                },
            )
            .unwrap()
            .note;
        notes.push(note.id);
    }
    notes.sort();
    // Index rebuilds in these tests reach the embedding branch; a real remote
    // workspace must have acknowledged before that branch may contact a service.
    {
        let library = app.library_mut(&identity).unwrap();
        let current = library.retrieval_settings().unwrap();
        let mut params = current.params;
        params.remote_consent = true;
        library
            .set_retrieval_params(
                &id(),
                pkms_core::SetRetrievalParams {
                    base_row_version: current.row_version,
                    params,
                },
            )
            .unwrap();
    }
    (root, app, identity, notes)
}
fn replace(app: &mut AppService, identity: &WorkspaceIdentity, object: &str, text: &str) {
    let note = app.library(identity).unwrap().note(object).unwrap();
    let paragraph = if text.is_empty() {
        json!({"type":"paragraph","attrs":{"id":id()}})
    } else {
        json!({"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":text}]})
    };
    app.library_mut(identity)
        .unwrap()
        .save_document(
            &id(),
            SaveNote {
                note_id: object.into(),
                base_revision_id: note.revision_id,
                title: note.title,
                document_schema: 2,
                document: serde_json::from_value(json!({"type":"doc","content":[paragraph]}))
                    .unwrap(),
            },
        )
        .unwrap();
}
fn claim(
    app: &mut AppService,
    identity: &WorkspaceIdentity,
    scope: SearchScope,
) -> (EmbeddingProfile, RebuildLease, InferenceConnection) {
    let library = app.library_mut(identity).unwrap();
    let profile = library
        .register_embedding_profile(
            &id(),
            pkms_inference::remote_chunks::default_profile_spec(
                "https://example.invalid/ai/v1".into(),
            ),
        )
        .unwrap()
        .profile;
    let job = library
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id.clone(),
                scope,
            },
        )
        .unwrap()
        .job;
    let lease = library.claim_index_rebuild(&job.id).unwrap().unwrap();
    let connection = app.inference_connection(identity).unwrap();
    (profile, lease, connection)
}
fn config(root: &std::path::Path) -> InferenceConfig {
    InferenceConfig {
        executable: root.join("unused.exe"),
        model_directory: root.join("models"),
        vector_root: root.join("vectors"),
        shutdown: Default::default(),
    }
}
fn generation_counts(root: &std::path::Path) -> (u32, u32) {
    let db = rusqlite::Connection::open_with_flags(
        root.join("knowledge/library.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    db.query_row(
        "SELECT count(*),coalesce(sum(status='running'),0) FROM index_generations",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

#[test]
fn whole_objects_share_a_bounded_batch_without_advancing_progress() {
    let (_root, mut app, identity, notes) = fixture(4, 12);
    let (profile, lease, mut connection) = claim(&mut app, &identity, SearchScope::default());
    let batch = connection
        .prepare_rebuild_batch(&profile, &lease, &notes, &mut |_| {})
        .unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch.iter().map(|o| o.plan.chunks.len()).sum::<usize>(), 24);
    assert_eq!(batch[0].plan.object_id, notes[0]);
    assert_eq!(batch[1].plan.object_id, notes[1]);
    assert_eq!(
        connection
            .library
            .index_rebuild_job(&lease.job.id)
            .unwrap()
            .processed,
        0
    );
}

#[test]
fn empty_entries_never_checkpoint_past_a_pending_object() {
    let (_root, mut app, identity, notes) = fixture(3, 1);
    replace(&mut app, &identity, &notes[1], "");
    let (profile, lease, mut connection) = claim(&mut app, &identity, SearchScope::default());
    let batch = connection
        .prepare_rebuild_batch(&profile, &lease, &notes, &mut |_| {})
        .unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].plan.object_id, notes[0]);
    assert_eq!(
        connection
            .library
            .index_rebuild_job(&lease.job.id)
            .unwrap()
            .processed,
        0
    );
    let scope = SearchScope {
        object_ids: notes[1..].to_vec(),
        ..Default::default()
    };
    let (profile, lease, mut connection) = claim(&mut app, &identity, scope);
    let batch = connection
        .prepare_rebuild_batch(&profile, &lease, &notes[1..], &mut |_| {})
        .unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].plan.object_id, notes[2]);
    let job = connection.library.index_rebuild_job(&lease.job.id).unwrap();
    assert_eq!((job.processed, job.skipped), (1, 1));
    assert_eq!(job.after_object_id.as_deref(), Some(notes[1].as_str()));
}

#[test]
fn oversized_object_stays_alone_and_covers_every_character() {
    let (_root, mut app, identity, notes) = fixture(2, 1);
    let original = "合成长文😀".repeat(8000);
    replace(&mut app, &identity, &notes[0], &original);
    let (profile, lease, mut connection) = claim(&mut app, &identity, SearchScope::default());
    let batch = connection
        .prepare_rebuild_batch(&profile, &lease, &notes, &mut |_| {})
        .unwrap();
    assert_eq!(batch.len(), 1);
    assert!(batch[0].plan.chunks.len() > 32);
    // Consecutive chunks of one block now repeat a tail on purpose, so the plan no
    // longer concatenates to the original. The contract is coverage: the ranges
    // must still cover every character of the block without a gap.
    let mut ranges = batch[0]
        .plan
        .chunks
        .iter()
        .map(|chunk| (chunk.start_codepoint, chunk.end_codepoint))
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    let mut covered = 0;
    for (start, end) in ranges {
        assert!(start <= covered, "index plan has a text gap");
        covered = covered.max(end);
    }
    assert_eq!(covered as usize, original.chars().count());
}

#[test]
fn revision_change_during_batch_setup_cleans_preceding_leases() {
    let (root, mut app, identity, notes) = fixture(3, 1);
    let (profile, lease, mut connection) = claim(&mut app, &identity, SearchScope::default());
    let batch = connection
        .prepare_rebuild_batch(&profile, &lease, &notes, &mut |_| {})
        .unwrap();
    replace(&mut app, &identity, &notes[1], "updated after preparation");
    let error = connection
        .publish_rebuild_batch(&config(root.path()), &profile, &lease, batch, &mut |_| {})
        .unwrap_err();
    assert!(matches!(
        error,
        AppError::Core(pkms_core::Error::InvalidInput("index plan input changed"))
    ));
    assert_eq!(generation_counts(root.path()), (1, 0));
    assert_eq!(
        connection
            .library
            .index_rebuild_job(&lease.job.id)
            .unwrap()
            .processed,
        0
    );
}

#[test]
fn missing_credentials_and_cancellation_end_every_unpublished_lease() {
    for cancel in [false, true] {
        let (root, mut app, identity, notes) = fixture(3, 1);
        let (profile, lease, mut connection) = claim(&mut app, &identity, SearchScope::default());
        let batch = connection
            .prepare_rebuild_batch(&profile, &lease, &notes, &mut |_| {})
            .unwrap();
        if cancel {
            connection
                .control
                .cancel_index_rebuild(&id(), &lease.job.id)
                .unwrap();
        }
        let error = connection
            .publish_rebuild_batch(&config(root.path()), &profile, &lease, batch, &mut |_| {})
            .unwrap_err();
        assert!(
            matches!(error,AppError::InvalidInput(code) if code == if cancel {"canceled"}else{"model_key_missing"})
        );
        assert_eq!(generation_counts(root.path()), (3, 0));
        assert_eq!(
            connection
                .library
                .index_rebuild_job(&lease.job.id)
                .unwrap()
                .processed,
            0
        );
        assert!(
            connection
                .library
                .active_index_generations()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn later_invalid_input_does_not_disable_the_first_valid_object() {
    let (_root, mut app, identity, notes) = fixture(3, 1);
    let note = app.library(&identity).unwrap().note(&notes[1]).unwrap();
    app.library_mut(&identity)
        .unwrap()
        .save_document(
            &id(),
            SaveNote {
                note_id: note.id,
                base_revision_id: note.revision_id,
                title: "😀".repeat(300),
                document_schema: note.schema_version,
                document: note.document,
            },
        )
        .unwrap();
    let (profile, lease, mut connection) = claim(&mut app, &identity, SearchScope::default());
    let batch = connection
        .prepare_rebuild_batch(&profile, &lease, &notes, &mut |_| {})
        .unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].plan.object_id, notes[0]);
    assert_eq!(
        connection
            .library
            .index_rebuild_job(&lease.job.id)
            .unwrap()
            .processed,
        0
    );
    let (profile, lease, mut connection) = claim(
        &mut app,
        &identity,
        SearchScope {
            object_ids: notes[1..].to_vec(),
            ..Default::default()
        },
    );
    assert!(matches!(
        connection.prepare_rebuild_batch(&profile, &lease, &notes[1..], &mut |_| {}),
        Err(AppError::InvalidInput("embedding_title_token_limit"))
    ));
}

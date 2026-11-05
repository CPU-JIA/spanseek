//! FA4-08: feedback stays attached to the question, the frozen fragments and the
//! retrieval parameters of the answer it was given for, through the IPC surface.
use pkms_app::*;
use pkms_core::{
    CompleteAnswer, GenerationProfile, SearchMode, SearchQuery, SearchScope, StartAnswer,
};
use serde_json::{Value, json};
use std::path::Path;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn workspace(app: &mut AppService, path: &Path) -> WorkspaceIdentity {
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, path).unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let result = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(result.status, OperationStatus::Succeeded);
    result.result.unwrap().identity
}

fn request(identity: &WorkspaceIdentity, command: Value) -> DesktopRequest {
    DesktopRequest::parse(json!({
        "api_version": 1, "request_id": id(), "identity": identity, "command": command,
    }))
    .unwrap()
}

fn feedback(answer_id: &str, rating: i32, comment: &str) -> Value {
    json!({"kind": "answers_feedback", "payload": {
        "command_id": id(),
        "request": {"answer_id": answer_id, "rating": rating, "comment": comment},
    }})
}

#[test]
fn feedback_ipc_keeps_the_question_fragments_and_change_time() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path()).unwrap();
    let identity = workspace(&mut app, &root.path().join("library"));

    // A completed answer is built through the core, then feedback is driven
    // through the same command the interface uses.
    let (answer_id, note_id) = {
        let library = app.library_mut(&identity).unwrap();
        let note = library.create_note("反馈链路", "副本保留七天。").unwrap();
        let accepted = library
            .accept_answer(
                &id(),
                StartAnswer {
                    max_output_tokens: None,
                    citation_count: Some(5),
                    agent: None,
                    search: SearchQuery {
                        query: "副本保留几天？".into(),
                        mode: SearchMode::Lexical,
                        scope: SearchScope::default(),
                        limit: 8,
                        minimum_cosine: None,
                    },
                    model: Some(GenerationProfile {
                        channel_id: None,
                        capabilities: None,
                        chat_token_limit: None,
                        provider: "ollama".into(),
                        endpoint: "http://127.0.0.1:11434".into(),
                        model: "test-fixture".into(),
                        model_digest: "fixture-digest".into(),
                        config_revision: 1,
                        temperature: 0.0,
                        context_tokens: 4096,
                    }),
                },
            )
            .unwrap();
        let lease = library.claim_answer(&accepted.answer.id).unwrap().unwrap();
        library
            .prepare_answer_evidence(&accepted.answer.id, &lease.lease_token, vec![])
            .unwrap();
        library
            .complete_answer(CompleteAnswer {
                answer_id: accepted.answer.id.clone(),
                lease_token: lease.lease_token,
                text: "副本保留七天 [E1]".into(),
                cited_ids: vec!["E1".into()],
            })
            .unwrap();
        (accepted.answer.id, note.id)
    };

    let reply = app
        .dispatch(request(&identity, feedback(&answer_id, -1, "片段不全")))
        .unwrap();
    let DesktopData::AnswerMutation(mutation) = reply.data else {
        panic!("feedback must answer with an answer mutation")
    };
    let saved = mutation.answer.feedback.expect("feedback is returned");
    assert_eq!(saved.rating, -1);
    assert_eq!(saved.comment, "片段不全");
    let changed_at = saved
        .updated_at
        .expect("the change time comes from the receipt");
    assert!(
        changed_at.len() >= 20 && changed_at.contains('T'),
        "{changed_at}"
    );
    // The answer still carries the question and the frozen fragment it referred to.
    assert_eq!(mutation.answer.request.search.query, "副本保留几天？");
    assert_eq!(mutation.answer.request.citation_count, Some(5));
    assert_eq!(mutation.answer.evidence[0].citation_id, "E1");
    assert_eq!(
        mutation.answer.evidence[0].evidence.target.object_id,
        note_id
    );

    // The change time is read only: a caller cannot set it, and an answer that is
    // still running cannot be rated at all.
    assert_eq!(
        app.dispatch(request(
            &identity,
            json!({"kind": "answers_feedback", "payload": {
                "command_id": id(),
                "request": {"answer_id": answer_id, "rating": 1, "comment": "",
                            "updated_at": changed_at},
            }}),
        ))
        .unwrap_err()
        .code,
        ApiErrorCode::InvalidInput
    );
    let queued = {
        let library = app.library_mut(&identity).unwrap();
        let accepted = library
            .accept_answer(
                &id(),
                StartAnswer {
                    max_output_tokens: None,
                    citation_count: None,
                    agent: None,
                    search: SearchQuery {
                        query: "副本保留几天？".into(),
                        mode: SearchMode::Lexical,
                        scope: SearchScope::default(),
                        limit: 8,
                        minimum_cosine: None,
                    },
                    model: None,
                },
            )
            .unwrap();
        accepted.answer.id
    };
    assert_eq!(
        app.dispatch(request(&identity, feedback(&queued, 1, "")))
            .unwrap_err()
            .code,
        ApiErrorCode::InvalidInput
    );

    // Feedback survives a reopen through the same read model.
    let reopened = app.library(&identity).unwrap().answer(&answer_id).unwrap();
    let restored = reopened.feedback.unwrap();
    assert_eq!(restored.comment, "片段不全");
    assert!(restored.updated_at.is_some());
    assert_eq!(reopened.cited_ids, vec!["E1".to_string()]);
}

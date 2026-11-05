//! Durable orchestration tests. The ignored integration requires real local E5 assets.
use pkms_app::{AppService, GrantPurpose, InferenceConfig, OperationKind, WorkspaceIdentity};
use pkms_core::*;
use std::{path::Path, time::Instant};
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn app(root: &Path) -> (AppService, WorkspaceIdentity) {
    let mut app = AppService::open(root.join("app")).unwrap();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, root.join("knowledge"))
        .unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let identity = app
        .execute_operation(&accepted.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity;
    (app, identity)
}
fn query(mode: SearchMode) -> SearchQuery {
    SearchQuery {
        query: "中文知识".into(),
        mode,
        scope: SearchScope::default(),
        limit: 8,
        minimum_cosine: None,
    }
}
fn allow_remote(app: &mut AppService, identity: &WorkspaceIdentity) {
    let library = app.library_mut(identity).unwrap();
    let current = library.retrieval_settings().unwrap();
    let mut params = current.params;
    params.remote_consent = true;
    library
        .set_retrieval_params(
            &id(),
            SetRetrievalParams {
                base_row_version: current.row_version,
                params,
            },
        )
        .unwrap();
}
fn config(root: &Path) -> InferenceConfig {
    InferenceConfig {
        executable: root.join("absent-worker.exe"),
        model_directory: root.join("models"),
        vector_root: root.join("vectors"),
        shutdown: Default::default(),
    }
}
#[test]
fn no_evidence_unconfigured_and_interrupted_answers_are_durable_without_worker() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = app(root.path());
    let absent = app
        .library_mut(&identity)
        .unwrap()
        .accept_answer_from_settings(&id(), query(SearchMode::Lexical))
        .unwrap()
        .answer
        .id;
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config(root.path()), &mut |_, _, _| {})
        .unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .answer(&absent)
            .unwrap()
            .status,
        AnswerStatus::NoEvidence
    );
    app.library_mut(&identity)
        .unwrap()
        .create_note("中文知识", "中文知识正文")
        .unwrap();
    let pending = app
        .library_mut(&identity)
        .unwrap()
        .accept_answer_from_settings(&id(), query(SearchMode::Lexical))
        .unwrap()
        .answer
        .id;
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config(root.path()), &mut |_, _, _| {})
        .unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .answer(&pending)
            .unwrap()
            .status,
        AnswerStatus::ModelUnconfigured
    );
    let interrupted = app
        .library_mut(&identity)
        .unwrap()
        .accept_answer_from_settings(&id(), query(SearchMode::Lexical))
        .unwrap()
        .answer
        .id;
    app.library_mut(&identity)
        .unwrap()
        .claim_answer(&interrupted)
        .unwrap()
        .unwrap();
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config(root.path()), &mut |_, _, _| {})
        .unwrap();
    let record = app
        .library(&identity)
        .unwrap()
        .answer(&interrupted)
        .unwrap();
    assert_eq!(record.status, AnswerStatus::Failed);
    assert_eq!(
        record.error_code.as_deref(),
        Some("interrupted_unknown_outcome")
    );
}
#[test]
#[ignore = "requires built runtime inference binary and PKMS_EMBEDDING_DIR with verified E5 model"]
fn real_e5_rebuild_search_and_revision_invalidation() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = app(root.path());
    let first = app
        .library_mut(&identity)
        .unwrap()
        .create_note("中文知识", "中文知识管理帮助用户组织阅读材料与笔记。")
        .unwrap();
    app.library_mut(&identity)
        .unwrap()
        .create_note("Rust并发", "Rust所有权模型保证内存安全。")
        .unwrap();
    let profile = app
        .library_mut(&identity)
        .unwrap()
        .register_embedding_profile(&id(), pkms_inference::profile::profile_spec())
        .unwrap()
        .profile;
    let rebuild = app
        .library_mut(&identity)
        .unwrap()
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id.clone(),
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let mut config = config(root.path());
    config.executable = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug/pkms-inference.exe")
        .canonicalize()
        .unwrap();
    config.model_directory = std::env::var_os("PKMS_EMBEDDING_DIR")
        .expect("PKMS_EMBEDDING_DIR")
        .into();
    let started = Instant::now();
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config.clone(), &mut |_, _, _| {})
        .unwrap();
    eprintln!("real rebuild_ms={}", started.elapsed().as_millis());
    let job = app
        .library(&identity)
        .unwrap()
        .index_rebuild_job(&rebuild.id)
        .unwrap();
    assert_eq!(job.status, RebuildStatus::Completed, "{:?}", job.error_code);
    assert_eq!(job.processed, 2);
    let mut connection = app.inference_connection(&identity).unwrap();
    let started = Instant::now();
    let found = connection
        .search(&config, query(SearchMode::Semantic))
        .unwrap();
    eprintln!("real warm_search_ms={}", started.elapsed().as_millis());
    assert!(found.semantic_available);
    assert!(!found.items.is_empty());
    assert!(
        found
            .items
            .iter()
            .any(|hit| hit.target.object_id == first.id)
    );
    // Local and remote profiles coexist in one identity space: switching away
    // records the local profile as the rollback target without touching vectors.
    let remote =
        pkms_inference::remote_chunks::default_profile_spec("https://example.invalid/v1".into());
    let switched = app
        .library_mut(&identity)
        .unwrap()
        .register_embedding_profile(&id(), remote)
        .unwrap()
        .profile;
    assert_ne!(switched.id, profile.id);
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .previous_embedding_profile()
            .unwrap()
            .unwrap()
            .id,
        profile.id
    );
    let reverted = app
        .library_mut(&identity)
        .unwrap()
        .revert_embedding_profile(&id())
        .unwrap();
    assert_eq!(reverted.profile.id, profile.id);
    assert_eq!(reverted.profile.spec.provider, None);

    let later = app
        .library_mut(&identity)
        .unwrap()
        .create_note("自动增量", "中文知识新增内容在保存后自动建立索引。")
        .unwrap();
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config.clone(), &mut |_, _, _| {})
        .unwrap();
    assert!(
        app.library(&identity)
            .unwrap()
            .active_index_generations()
            .unwrap()
            .iter()
            .any(|generation| generation.object_id == later.id),
        "a new note must be indexed without an explicit rebuild request"
    );
    let current = app
        .library(&identity)
        .unwrap()
        .object_metadata(&first.id)
        .unwrap();
    app.library_mut(&identity)
        .unwrap()
        .set_object_trashed(
            &id(),
            SetObjectFlag {
                object_id: first.id.clone(),
                base_row_version: current.row_version,
                value: true,
            },
        )
        .unwrap();
    let found = connection
        .search(&config, query(SearchMode::Hybrid))
        .unwrap();
    assert!(
        found
            .items
            .iter()
            .all(|hit| hit.target.object_id != first.id)
    );
    pkms_app::stop_inference_session();
}

#[test]
fn automatic_index_failure_is_durable_and_backed_off() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = app(root.path());
    app.library_mut(&identity)
        .unwrap()
        .create_note("中文知识", "自动索引正文")
        .unwrap();
    allow_remote(&mut app, &identity);
    app.library_mut(&identity)
        .unwrap()
        .register_embedding_profile(
            &id(),
            pkms_inference::remote_chunks::default_profile_spec(
                "https://example.invalid/ai/v1".into(),
            ),
        )
        .unwrap();
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config(root.path()), &mut |_, _, _| {})
        .unwrap();
    let jobs = app
        .library(&identity)
        .unwrap()
        .index_rebuild_jobs_page(None, 100)
        .unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, RebuildStatus::Failed);
    assert_eq!(jobs[0].error_code.as_deref(), Some("model_key_missing"));
    assert!(
        app.library(&identity)
            .unwrap()
            .dirty_index_object_ids(32)
            .unwrap()
            .is_empty()
    );
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config(root.path()), &mut |_, _, _| {})
        .unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .index_rebuild_jobs_page(None, 100)
            .unwrap()
            .len(),
        1,
        "immediate next wake must not retry a backed-off input"
    );
}

#[test]
#[ignore = "requires verified qwen3.5:4b at isolated Ollama 11435 and protocol 2 runtime worker"]
fn real_ollama_answer_persists_evidence_and_valid_citation() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = app(root.path());
    app.library_mut(&identity)
        .unwrap()
        .create_note(
            "蓝桥备份规则",
            "蓝桥知识库每天晚上九点进行完整备份，保留最近七天的备份。",
        )
        .unwrap();
    let profile = GenerationProfile {
        channel_id: None,
        capabilities: None,
        chat_token_limit: None,
        provider: "ollama".into(),
        endpoint: std::env::var("PKMS_OLLAMA_ENDPOINT")
            .unwrap_or_else(|_| "http://127.0.0.1:11435".into()),
        model: std::env::var("PKMS_OLLAMA_MODEL").unwrap_or_else(|_| "qwen3.5:4b".into()),
        model_digest: std::env::var("PKMS_OLLAMA_DIGEST").unwrap_or_else(|_| {
            "2a654d98e6fba55d452b7043684e9b57a947e393bbffa62485a7aac05ee4eefd".into()
        }),
        config_revision: 1,
        temperature: 0.0,
        context_tokens: 8192,
    };
    pkms_app::verify_generation_model(&profile, &Default::default()).unwrap();
    let base = app
        .library(&identity)
        .unwrap()
        .generation_settings()
        .unwrap()
        .row_version;
    app.library_mut(&identity)
        .unwrap()
        .set_generation_profile(
            &id(),
            SetGenerationProfile {
                base_row_version: base,
                profile: Some(profile),
            },
        )
        .unwrap();
    let mut search = query(SearchMode::Lexical);
    search.query = "蓝桥知识库 完整备份".into();
    let answer = app
        .library_mut(&identity)
        .unwrap()
        .accept_answer_from_settings(&id(), search)
        .unwrap()
        .answer;
    let mut config = config(root.path());
    config.executable = std::env::var_os("PKMS_INFERENCE_EXE")
        .map(Into::into)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/pkms-inference.exe")
                .canonicalize()
                .unwrap()
        });
    let mut updates = 0;
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config.clone(), &mut |_, answer, _| {
            if let Some(answer) = answer {
                if answer.status == AnswerStatus::Running {
                    assert!(answer.text.is_empty(), "wire JSON must never be persisted");
                }
                updates += 1;
            }
        })
        .unwrap();
    let result = app.library(&identity).unwrap().answer(&answer.id).unwrap();
    assert_eq!(
        result.status,
        AnswerStatus::Completed,
        "{:?}: {}",
        result.error_code,
        result.text
    );
    assert!(result.evidence_current);
    assert!(!result.evidence.is_empty());
    assert!(!result.cited_ids.is_empty());
    assert!(
        result.text.contains("九点") || result.text.contains("9"),
        "{}",
        result.text
    );
    assert!(
        updates >= 3,
        "durable claim/evidence/output/completion notifications"
    );
    eprintln!("actual local answer: {}", result.text);
    let next = app
        .library_mut(&identity)
        .unwrap()
        .accept_answer_from_settings(&id(), result.request.search.clone())
        .unwrap()
        .answer;
    let worker = app.inference_jobs_worker().unwrap().unwrap();
    let mut canceled = false;
    let mut evidence_progress = 0;
    worker
        .run(config.clone(), &mut |_, answer, _| {
            if answer.is_some_and(|a| {
                a.id == next.id && a.status == AnswerStatus::Running && !a.evidence.is_empty()
            }) {
                evidence_progress += 1;
            }
            // First notification freezes evidence; the second reports an actual
            // first wire token, buffered privately by the structured adapter.
            if !canceled && evidence_progress == 2 {
                app.library_mut(&identity)
                    .unwrap()
                    .cancel_answer(&id(), &next.id)
                    .unwrap();
                canceled = true;
            }
        })
        .unwrap();
    assert!(canceled, "cancel must follow a real buffered model token");
    let canceled = app.library(&identity).unwrap().answer(&next.id).unwrap();
    assert_eq!(canceled.status, AnswerStatus::Canceled);
    assert!(
        canceled.text.is_empty(),
        "cancellation must not expose partial JSON"
    );
    // Ensure lexical retrieval has the requested topic, while the immutable
    // evidence deliberately contains no brand value. This tests model refusal,
    // not the separate zero-hit path already covered above.
    app.library_mut(&identity)
        .unwrap()
        .create_note(
            "蓝桥知识库 完整备份 使用哪个品牌的硬盘？",
            "蓝桥知识库每天晚上九点进行完整备份。资料未提供硬盘品牌。",
        )
        .unwrap();
    let mut missing_query = result.request.search;
    missing_query.query = "蓝桥知识库 完整备份 使用哪个品牌的硬盘？".into();
    let missing = app
        .library_mut(&identity)
        .unwrap()
        .accept_answer_from_settings(&id(), missing_query)
        .unwrap()
        .answer;
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config.clone(), &mut |_, answer, _| {
            if let Some(answer) = answer {
                assert!(
                    answer.text.is_empty() || answer.text == INSUFFICIENT_EVIDENCE_MARKER,
                    "refusal must never persist wire JSON"
                );
            }
        })
        .unwrap();
    let missing = app.library(&identity).unwrap().answer(&missing.id).unwrap();
    assert!(
        !missing.evidence.is_empty(),
        "model refusal must use real frozen evidence"
    );
    assert_eq!(missing.status, AnswerStatus::NoEvidence);
    assert_eq!(missing.error_code.as_deref(), Some("insufficient_evidence"));
    assert_eq!(missing.text, INSUFFICIENT_EVIDENCE_MARKER);
    pkms_app::stop_inference_session();
}

#[test]
fn local_profile_without_worker_fails_once_without_busy_loop() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = app(root.path());
    app.library_mut(&identity)
        .unwrap()
        .create_note("旧配置", "保留原文")
        .unwrap();
    app.library_mut(&identity)
        .unwrap()
        .register_embedding_profile(&id(), pkms_inference::profile::profile_spec())
        .unwrap();
    let started = Instant::now();
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config(root.path()), &mut |_, _, _| {})
        .unwrap();
    assert!(started.elapsed().as_secs() < 5);
    let jobs = app
        .library(&identity)
        .unwrap()
        .index_rebuild_jobs_page(None, 100)
        .unwrap();
    assert_eq!(
        jobs.len(),
        1,
        "one durable attempt, not a busy loop: {jobs:?}"
    );
    assert_eq!(jobs[0].status, RebuildStatus::Failed);
    assert!(
        jobs[0]
            .error_code
            .as_deref()
            .is_some_and(|code| code != "embedding_remote_required"),
        "a missing worker is an environment failure, not 'remote required': {:?}",
        jobs[0].error_code
    );
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .active_embedding_profile()
            .unwrap()
            .unwrap()
            .spec
            .provider,
        None,
        "the local profile stays active for retry after the worker is repaired"
    );
}

#[test]
#[ignore = "requires an OpenAI-compatible llama.cpp server on 127.0.0.1:11436 with a loaded GGUF model"]
fn loopback_openai_compatible_generation_is_local_and_completes_a_chat_run() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = app(root.path());
    let model = std::env::var("PKMS_LOCAL_OPENAI_MODEL").unwrap_or_else(|_| {
        "E:\\tmp\\User\\pkms-v194\\ollama-models\\blobs\\sha256-7f4030143c1c477224c5434f8272c662a8b042079a0a584f0a27a1684fe2e1fa".into()
    });
    let profile = GenerationProfile {
        channel_id: None,
        capabilities: None,
        chat_token_limit: Some(ChatTokenLimit::MaxTokens),
        provider: "openai_chat".into(),
        endpoint: "http://127.0.0.1:11436/v1".into(),
        model,
        model_digest: String::new(),
        config_revision: 1,
        temperature: 0.0,
        context_tokens: 4096,
    };
    let base = app
        .library(&identity)
        .unwrap()
        .generation_settings()
        .unwrap()
        .row_version;
    app.library_mut(&identity)
        .unwrap()
        .set_generation_profile(
            &id(),
            SetGenerationProfile {
                base_row_version: base,
                profile: Some(profile),
            },
        )
        .unwrap();
    assert!(
        !app.library(&identity)
            .unwrap()
            .retrieval_settings()
            .unwrap()
            .params
            .remote_consent,
        "the test must prove loopback generation does not require remote consent"
    );
    let conversation = app
        .library_mut(&identity)
        .unwrap()
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "本机生成验收".into(),
            },
        )
        .unwrap()
        .conversation;
    let run = app
        .library_mut(&identity)
        .unwrap()
        .accept_chat_run_from_settings(
            &id(),
            StartChatRun {
                conversation_id: conversation.id.clone(),
                base_row_version: conversation.row_version,
                text: "用一句中文回答：本机模型可用吗？".into(),
                purpose: ChatPurpose::Chat,
                materials: Vec::new(),
                scope: SearchScope::default(),
                use_knowledge_tools: false,
                options: Some(ChatRunOptions {
                    max_model_calls: 2,
                    max_tool_calls: 1,
                    max_output_tokens: 256,
                    total_timeout_seconds: 120,
                }),
            },
        )
        .unwrap()
        .run;
    let mut config = config(root.path());
    config.executable = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug/pkms-inference.exe")
        .canonicalize()
        .unwrap();
    let started = Instant::now();
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config, &mut |_, _, _| {})
        .unwrap();
    let result = app.library(&identity).unwrap().chat_run(&run.id).unwrap();
    assert_eq!(
        result.status,
        ChatRunStatus::Completed,
        "{:?}: {}",
        result.error_code,
        result.finish_reason.as_deref().unwrap_or("")
    );
    let messages = app
        .library(&identity)
        .unwrap()
        .chat_messages(&conversation.id, 0, 10)
        .unwrap();
    let answer = messages
        .iter()
        .find(|message| message.role == ChatRole::Assistant)
        .and_then(|message| (!message.content.trim().is_empty()).then_some(message.content.clone()))
        .expect("assistant answer text");
    eprintln!(
        "local_openai_chat_ms={} answer={}",
        started.elapsed().as_millis(),
        answer
    );
}

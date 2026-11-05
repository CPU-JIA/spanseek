//! Opt-in synthetic end-to-end BGE retrieval -> DeepSeek answer quality export.
//! Gold labels are exported for review but never sent to either model.
use pkms_app::{
    AppService, GrantPurpose, InferenceConfig, OperationKind, WorkspaceIdentity,
    inference_credentials::{self as vault, CredentialBinding},
};
use pkms_core::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, io::Write, path::PathBuf, time::Instant};
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap()
}
struct Credentials(Vec<CredentialBinding>);
impl Drop for Credentials {
    fn drop(&mut self) {
        for binding in &self.0 {
            let _ = vault::delete(binding);
        }
    }
}
fn copy_credential(
    identity: &WorkspaceIdentity,
    source: (&str, &str),
    protocol: &str,
    endpoint: &str,
    model: &str,
    cleanup: &mut Credentials,
) {
    let from = CredentialBinding::new(source.0, source.1, protocol, endpoint, model).unwrap();
    let secret = vault::load(&from)
        .unwrap()
        .expect("explicit synthetic vault credential required");
    let to = CredentialBinding::new(
        &identity.local_instance_id,
        &identity.workspace_id,
        protocol,
        endpoint,
        model,
    )
    .unwrap();
    vault::store(&to, &secret).unwrap();
    cleanup.0.push(to);
}
#[test]
#[ignore = "requires real BGE/DeepSeek QA vault bindings, runtime, fresh PKMS_ANSWER_QUALITY_OUTPUT"]
fn export_real_development_answers() {
    export_real_answers("development");
}

#[test]
#[ignore = "requires real QA vault credentials, runtime and a fresh output; reserved labels are never model input"]
fn export_real_reserved_answers() {
    export_real_answers("reserved");
}

fn export_real_answers(split: &str) {
    let root = PathBuf::from(std::env::var_os("PKMS_ANSWER_QUALITY_OUTPUT").unwrap());
    assert!(
        !root.exists(),
        "new output required; preserve previous results"
    );
    fs::create_dir_all(&root).unwrap();
    let (fixture_bytes, expected_hash): (&[u8], &str) = match split {
        "development" => (
            include_bytes!("../../../scripts/w4-quality/fixtures/development.json"),
            "77b0b5e43320fa1829e3f78bd0ae45b36166e3addbd6ae6a47b1be76cb983c7b",
        ),
        "reserved" => (
            include_bytes!("../../../scripts/w4-quality/fixtures/reserved.json"),
            "42bb03e02f0aaa72acf394d708838359a7ac594ced7be59aa9cd39e9dc87fd43",
        ),
        _ => panic!("unsupported fixture split"),
    };
    let fixture_hash = format!("{:x}", Sha256::digest(fixture_bytes));
    assert_eq!(fixture_hash, expected_hash);
    let fixture: Value = serde_json::from_slice(fixture_bytes).unwrap();
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
    let mut mapping = HashMap::new();
    for doc in fixture["documents"].as_array().unwrap() {
        let note = app
            .library_mut(&identity)
            .unwrap()
            .create_note(text(doc, "title"), text(doc, "text"))
            .unwrap();
        mapping.insert(text(doc,"id").to_owned(),json!({"object_id":note.id,"revision_id":note.revision_id,"content_hash":note.content_hash}));
    }
    let empty = app
        .library_mut(&identity)
        .unwrap()
        .create_collection("Empty synthetic quality scope")
        .unwrap();
    let endpoint = std::env::var("PKMS_QUALITY_EMBEDDING_ENDPOINT").unwrap();
    let bge_source = (
        std::env::var("PKMS_QUALITY_CREDENTIAL_INSTANCE").unwrap(),
        std::env::var("PKMS_QUALITY_CREDENTIAL_WORKSPACE").unwrap(),
    );
    let gen_source = (
        std::env::var("PKMS_QUALITY_GENERATION_INSTANCE").unwrap(),
        std::env::var("PKMS_QUALITY_GENERATION_WORKSPACE").unwrap(),
    );
    let mut cleanup = Credentials(Vec::new());
    copy_credential(
        &identity,
        (&bge_source.0, &bge_source.1),
        "openai_embeddings",
        &endpoint,
        pkms_inference::remote_chunks::MODEL,
        &mut cleanup,
    );
    copy_credential(
        &identity,
        (&gen_source.0, &gen_source.1),
        "openai_chat",
        "https://api.deepseek.com",
        "deepseek-flash",
        &mut cleanup,
    );
    let db = app.library_mut(&identity).unwrap();
    let profile = db
        .register_embedding_profile(
            &id(),
            pkms_inference::remote_chunks::default_profile_spec(endpoint),
        )
        .unwrap()
        .profile;
    let generation = GenerationProfile {
        channel_id: None,
        capabilities: None,
        provider: "openai_chat".into(),
        endpoint: "https://api.deepseek.com".into(),
        model: "deepseek-flash".into(),
        model_digest: String::new(),
        config_revision: 1,
        temperature: 0.,
        context_tokens: 65536,
        chat_token_limit: Some(ChatTokenLimit::MaxTokens),
    };
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 0,
            profile: Some(generation.clone()),
        },
    )
    .unwrap();
    let rebuild = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id.clone(),
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let config = InferenceConfig {
        executable: PathBuf::from(std::env::var_os("PKMS_QUALITY_RUNTIME").unwrap())
            .canonicalize()
            .unwrap(),
        model_directory: root.join("unused-models"),
        vector_root: root.join("vectors"),
        shutdown: Default::default(),
    };
    let started = Instant::now();
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config.clone(), &mut |_, _, _| {})
        .unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .index_rebuild_job(&rebuild.id)
            .unwrap()
            .status,
        RebuildStatus::Completed
    );
    let provenance = json!({"fixture_split":split,"fixture_sha256":fixture_hash,"identity":identity,"mapping":mapping,"embedding_profile":profile,"generation_profile":generation,"max_output_tokens":8192,"build":"debug App + release worker","worker_sha256":format!("{:x}",Sha256::digest(fs::read(&config.executable).unwrap())),"index_ms":started.elapsed().as_millis(),"prompt_sha256":format!("{:x}",Sha256::digest(include_bytes!("../src/inference_answer_prompt.txt"))),"scope":"frozen synthetic 50 questions; production hybrid answer acceptance/worker; not a performance benchmark"});
    fs::write(
        root.join("provenance.json"),
        serde_json::to_vec_pretty(&provenance).unwrap(),
    )
    .unwrap();
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("answers.jsonl"))
        .unwrap();
    for query in fixture["queries"].as_array().unwrap() {
        let category = text(query, "category");
        let changed = if matches!(category, "disabled" | "trashed") {
            let key = query["excluded_object_keys"][0].as_str().unwrap();
            let object = text(&mapping[key], "object_id").to_owned();
            let metadata = app
                .library(&identity)
                .unwrap()
                .object_metadata(&object)
                .unwrap();
            let request = SetObjectFlag {
                object_id: object.clone(),
                base_row_version: metadata.row_version,
                value: category == "trashed",
            };
            if category == "trashed" {
                app.library_mut(&identity)
                    .unwrap()
                    .set_object_trashed(&id(), request)
                    .unwrap();
            } else {
                app.library_mut(&identity)
                    .unwrap()
                    .set_object_search_enabled(&id(), request)
                    .unwrap();
            }
            Some(object)
        } else {
            None
        };
        let scope = SearchScope {
            object_ids: query["scope"]["object_keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|key| text(&mapping[key.as_str().unwrap()], "object_id").to_owned())
                .collect(),
            collection_id: (category == "empty_scope").then(|| empty.id.clone()),
            ..Default::default()
        };
        let request = pkms_app::DesktopRequest {
            api_version: 1,
            request_id: id(),
            identity: Some(identity.clone()),
            command: pkms_app::DesktopCommand::AnswersStart {
                command_id: id(),
                search: SearchQuery {
                    query: text(query, "question").into(),
                    mode: SearchMode::Hybrid,
                    scope,
                    limit: 8,
                    minimum_cosine: None,
                },
                max_output_tokens: Some(8192),
            },
        };
        let reply = app.dispatch(request.clone()).unwrap();
        let pkms_app::DesktopData::AnswerMutation(accepted) = reply.data else {
            panic!("wrong acceptance reply")
        };
        let started = Instant::now();
        let mut stages = Vec::new();
        app.inference_jobs_worker()
            .unwrap()
            .unwrap()
            .run_with_progress(config.clone(), &mut |_, _, _| {}, &mut |_, stage| {
                stages.push(stage.clone())
            })
            .unwrap();
        let answer = app
            .library(&identity)
            .unwrap()
            .answer(&accepted.answer.id)
            .unwrap();
        let replay = app.dispatch(request).unwrap();
        let pkms_app::DesktopData::AnswerMutation(replay) = replay.data else {
            panic!("wrong replay reply")
        };
        assert!(replay.receipt.replayed);
        assert_eq!(replay.answer.id, answer.id);
        let mut hard_errors = Vec::new();
        for item in &answer.evidence {
            let key = mapping
                .iter()
                .find(|(_, v)| text(v, "object_id") == item.evidence.target.object_id)
                .map(|(k, _)| k);
            match key {
                None => hard_errors.push("unknown_object"),
                Some(key) => {
                    let allowed = query["scope"]["object_keys"].as_array().unwrap();
                    if !allowed.is_empty()
                        && !allowed.iter().any(|v| v.as_str() == Some(key.as_str()))
                    {
                        hard_errors.push("outside_scope");
                    }
                    if query["excluded_object_keys"]
                        .as_array()
                        .is_some_and(|v| v.iter().any(|v| v.as_str() == Some(key.as_str())))
                    {
                        hard_errors.push("excluded_object");
                    }
                }
            }
        }
        if !answer.evidence.is_empty() && !answer.evidence_current {
            hard_errors.push("stale_evidence");
        }
        if answer
            .cited_ids
            .iter()
            .any(|id| !answer.evidence.iter().any(|e| e.citation_id == *id))
        {
            hard_errors.push("unknown_citation");
        }
        if matches!(answer.status, AnswerStatus::Queued | AnswerStatus::Running) {
            hard_errors.push("not_terminal");
        }
        if !matches!(
            answer.status,
            AnswerStatus::Completed | AnswerStatus::NoEvidence
        ) {
            hard_errors.push("answer_failed");
        }
        writeln!(output,"{}",json!({"transport":"real_pkms_app","fixture_split":split,"fixture_sha256":fixture_hash,"query_id":query["id"],"category":category,"elapsed_ms":started.elapsed().as_secs_f64()*1000.,"answer":answer,"stages":stages,"hard_errors":hard_errors,"idempotent_replay":true})).unwrap();
        output.flush().unwrap();
        eprintln!(
            "{} status={:?} evidence={} elapsed_ms={} errors={:?}",
            text(query, "id"),
            answer.status,
            answer.evidence.len(),
            started.elapsed().as_millis(),
            hard_errors
        );
        if let Some(object) = changed {
            let metadata = app
                .library(&identity)
                .unwrap()
                .object_metadata(&object)
                .unwrap();
            let request = SetObjectFlag {
                object_id: object,
                base_row_version: metadata.row_version,
                value: category != "trashed",
            };
            if category == "trashed" {
                app.library_mut(&identity)
                    .unwrap()
                    .set_object_trashed(&id(), request)
                    .unwrap();
            } else {
                app.library_mut(&identity)
                    .unwrap()
                    .set_object_search_enabled(&id(), request)
                    .unwrap();
            }
            app.inference_jobs_worker()
                .unwrap()
                .unwrap()
                .run(config.clone(), &mut |_, _, _| {})
                .unwrap();
        }
    }
    for (key, value) in &mapping {
        let current = app
            .library(&identity)
            .unwrap()
            .note(text(value, "object_id"))
            .unwrap();
        assert_eq!(
            current.revision_id,
            text(value, "revision_id"),
            "source revision changed: {key}"
        );
        assert_eq!(
            current.content_hash,
            text(value, "content_hash"),
            "source changed: {key}"
        );
    }
    fs::write(
        root.join("sources-unchanged.json"),
        serde_json::to_vec_pretty(
            &json!({"notes":mapping.len(),"all_content_hashes_and_revisions_unchanged":true}),
        )
        .unwrap(),
    )
    .unwrap();
    pkms_app::stop_inference_session();
}

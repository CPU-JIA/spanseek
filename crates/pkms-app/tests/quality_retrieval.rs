//! Opt-in real App/BGE-M3 development-set export. Never runs against user libraries.
use pkms_app::{AppService, GrantPurpose, InferenceConfig, OperationKind};
use pkms_core::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, io::Write, path::PathBuf, time::Instant};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap()
}

#[test]
#[ignore = "requires BGE-M3 vault credential/runtime and a fresh PKMS_QUALITY_OUTPUT directory"]
fn export_real_development_retrieval() {
    export_real_retrieval("development");
}

#[test]
#[ignore = "requires real QA vault credentials, runtime and a fresh output; reserved labels are never model input"]
fn export_real_reserved_retrieval() {
    export_real_retrieval("reserved");
}

fn export_real_retrieval(split: &str) {
    let root = PathBuf::from(std::env::var_os("PKMS_QUALITY_OUTPUT").unwrap());
    assert!(
        !root.exists(),
        "preserve earlier evidence; choose a fresh directory"
    );
    fs::create_dir_all(&root).unwrap();
    let (fixture_bytes, expected_hash): (&[u8], &str) = match split {
        "development" => (
            include_bytes!("fixtures/quality/development.json"),
            "77b0b5e43320fa1829e3f78bd0ae45b36166e3addbd6ae6a47b1be76cb983c7b",
        ),
        "reserved" => (
            include_bytes!("fixtures/quality/reserved.json"),
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
    let mut reverse = HashMap::new();
    for document in fixture["documents"].as_array().unwrap() {
        assert_eq!(document["kind"], "note");
        assert_eq!(document["enabled"], true);
        assert_eq!(document["trashed"], false);
        let note = app
            .library_mut(&identity)
            .unwrap()
            .create_note(text(document, "title"), text(document, "text"))
            .unwrap();
        let input = app
            .library(&identity)
            .unwrap()
            .index_input(&note.id)
            .unwrap();
        assert_eq!(
            input.blocks.len(),
            1,
            "fixture requires one actual paragraph"
        );
        assert_eq!(input.blocks[0].text, text(document, "text"));
        let key = text(document, "id").to_owned();
        reverse.insert(note.id.clone(), key.clone());
        mapping.insert(key,json!({"object_id":note.id,"content_revision":note.revision_id,"block_id":input.blocks[0].target.block_id,"fixture_revision_key":document["revision_key"],"fixture_block_key":document["block_key"]}));
    }
    let empty_collection = app
        .library_mut(&identity)
        .unwrap()
        .create_collection("Quality empty scope")
        .unwrap();
    let config = InferenceConfig {
        executable: PathBuf::from(std::env::var_os("PKMS_QUALITY_RUNTIME").unwrap())
            .canonicalize()
            .unwrap(),
        model_directory: root.join("unused-models"),
        vector_root: root.join("vectors"),
        shutdown: Default::default(),
    };
    let runtime_sha = format!(
        "{:x}",
        Sha256::digest(fs::read(&config.executable).unwrap())
    );
    let endpoint = std::env::var("PKMS_QUALITY_EMBEDDING_ENDPOINT").unwrap();
    use pkms_app::inference_credentials::{self as vault, CredentialBinding};
    let source = CredentialBinding::new(
        &std::env::var("PKMS_QUALITY_CREDENTIAL_INSTANCE").unwrap(),
        &std::env::var("PKMS_QUALITY_CREDENTIAL_WORKSPACE").unwrap(),
        "openai_embeddings",
        &endpoint,
        pkms_inference::remote_chunks::MODEL,
    )
    .unwrap();
    let secret = vault::load(&source)
        .unwrap()
        .expect("synthetic QA credential is required");
    let binding = CredentialBinding::new(
        &identity.local_instance_id,
        &identity.workspace_id,
        "openai_embeddings",
        &endpoint,
        pkms_inference::remote_chunks::MODEL,
    )
    .unwrap();
    vault::store(&binding, &secret).unwrap();
    struct CredentialCleanup(CredentialBinding);
    impl Drop for CredentialCleanup {
        fn drop(&mut self) {
            let _ = vault::delete(&self.0);
        }
    }
    let _cleanup = CredentialCleanup(binding);
    let profile = app
        .library_mut(&identity)
        .unwrap()
        .register_embedding_profile(
            &id(),
            pkms_inference::remote_chunks::default_profile_spec(endpoint),
        )
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
    let started = Instant::now();
    app.inference_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config.clone(), &mut |_, _, _| {})
        .unwrap();
    let rebuild_ms = started.elapsed().as_millis();
    let job = app
        .library(&identity)
        .unwrap()
        .index_rebuild_job(&rebuild.id)
        .unwrap();
    assert_eq!(job.status, RebuildStatus::Completed, "{:?}", job.error_code);
    assert_eq!(job.processed, mapping.len() as u32);
    let configuration = json!({"profile":profile,"runtime_path":config.executable,"runtime_sha256":runtime_sha,"model_directory":config.model_directory,"limit":8,"minimum_cosine":null,"build":std::env::var("PKMS_QUALITY_BUILD").unwrap_or_else(|_| "debug".to_owned()),"warm_state":"same process and resident inference session after actual rebuild; unrelated host workloads present"});
    fs::write(root.join("provenance.json"),serde_json::to_vec_pretty(&json!({"fixture_split":split,"fixture_sha256":fixture_hash,"workspace_identity":identity,"configuration":configuration,"mapping":mapping,"empty_collection_id":empty_collection.id,"rebuild":job,"rebuild_ms":rebuild_ms,"index_status":app.library(&identity).unwrap().index_status().unwrap()})).unwrap()).unwrap();
    let mut output = fs::File::create(root.join("retrieval.jsonl")).unwrap();
    let mut connection = app.inference_connection(&identity).unwrap();
    for query in fixture["queries"].as_array().unwrap() {
        let category = text(query, "category");
        let changed = if matches!(category, "disabled" | "trashed") {
            let key = query["excluded_object_keys"][0].as_str().unwrap();
            let object_id = text(&mapping[key], "object_id").to_owned();
            let metadata = app
                .library(&identity)
                .unwrap()
                .object_metadata(&object_id)
                .unwrap();
            let request = SetObjectFlag {
                object_id: object_id.clone(),
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
            Some(object_id)
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
            collection_id: if category == "empty_scope" {
                Some(empty_collection.id.clone())
            } else {
                None
            },
            ..Default::default()
        };
        for (mode_name, mode) in [
            ("lexical", SearchMode::Lexical),
            ("semantic", SearchMode::Semantic),
            ("hybrid", SearchMode::Hybrid),
        ] {
            let request = SearchQuery {
                query: text(query, "question").into(),
                mode,
                scope: scope.clone(),
                limit: 8,
                minimum_cosine: None,
            };
            let started = Instant::now();
            let result = connection.search(&config, request.clone()).unwrap();
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            let active = app
                .library(&identity)
                .unwrap()
                .active_index_generations_in_scope(&scope)
                .unwrap();
            let retrieval_failure =
                mode != SearchMode::Lexical && !active.is_empty() && !result.semantic_available;
            if mode == SearchMode::Semantic && active.is_empty() {
                assert!(
                    result.items.is_empty(),
                    "empty authoritative scope must stay empty"
                );
            }
            let mut items = Vec::new();
            for hit in &result.items {
                let key = reverse
                    .get(&hit.target.object_id)
                    .map(String::as_str)
                    .unwrap_or("unknown-object");
                let mapped = mapping.get(key);
                let current = app
                    .library(&identity)
                    .unwrap()
                    .index_input(&hit.target.object_id);
                let eligible = current.is_ok()
                    && (scope.object_ids.is_empty()
                        || scope.object_ids.contains(&hit.target.object_id))
                    && scope.collection_id.is_none();
                let locator = current.as_ref().is_ok_and(|input| {
                    input.content_revision == hit.target.content_revision
                        && input.metadata_revision == hit.target.metadata_revision
                        && input.blocks.iter().any(|block| {
                            block.target.block_id == hit.target.block_id
                                && hit.target.end_codepoint >= hit.target.start_codepoint
                                && hit.target.end_codepoint as usize <= block.text.chars().count()
                                && block
                                    .text
                                    .chars()
                                    .skip(hit.target.start_codepoint as usize)
                                    .take(
                                        (hit.target.end_codepoint - hit.target.start_codepoint)
                                            as usize,
                                    )
                                    .collect::<String>()
                                    == hit.text
                        })
                });
                let generation_current = if let Some(generation_id) = &hit.generation_id {
                    active.iter().any(|generation| {
                        &generation.id == generation_id
                            && generation.object_id == hit.target.object_id
                            && generation.content_revision == hit.target.content_revision
                            && generation.metadata_revision == hit.target.metadata_revision
                            && generation.profile_id == profile.id
                    })
                } else {
                    mode == SearchMode::Lexical
                        || (mode == SearchMode::Hybrid && hit.lexical_rank.is_some())
                };
                let revision_key = mapped
                    .filter(|entry| text(entry, "content_revision") == hit.target.content_revision)
                    .map(|entry| text(entry, "fixture_revision_key"))
                    .unwrap_or("unknown-revision");
                let block_key = mapped
                    .filter(|entry| text(entry, "block_id") == hit.target.block_id)
                    .map(|entry| text(entry, "fixture_block_key"))
                    .unwrap_or("unknown-block");
                items.push(json!({"document_key":key,"revision_key":revision_key,"block_key":block_key,"authoritative_eligible":eligible,"generation_current":generation_current && locator,"locator_verified":locator,"actual_evidence":hit}));
            }
            let record = json!({"fixture_split":split,"fixture_sha256":fixture_hash,"transport":"real_pkms_app","workspace_identity":identity,"configuration":configuration,"mode":mode_name,"query_id":query["id"],"request":request,"items":items,"elapsed_ms":elapsed_ms,"retrieval_failure":retrieval_failure,"semantic_expected":!active.is_empty(),"availability":{"lexical":result.lexical_available,"semantic":result.semantic_available,"reason":result.unavailable_reason},"actual_lifecycle_change":changed});
            writeln!(output, "{}", serde_json::to_string(&record).unwrap()).unwrap();
            output.flush().unwrap();
            eprintln!(
                "{} {} items={} elapsed_ms={:.1} unavailable={:?}",
                text(query, "id"),
                mode_name,
                result.items.len(),
                elapsed_ms,
                result.unavailable_reason
            );
        }
        if let Some(object_id) = changed {
            let metadata = app
                .library(&identity)
                .unwrap()
                .object_metadata(&object_id)
                .unwrap();
            let request = SetObjectFlag {
                object_id,
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
    pkms_app::stop_inference_session();
}

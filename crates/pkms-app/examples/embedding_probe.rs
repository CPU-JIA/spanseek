//! Explicit live BGE-M3 integration on a new synthetic workspace. Secrets stay in the OS vault.
use pkms_app::inference_credentials::{self as vault, CredentialBinding};
use pkms_app::{AppService, GrantPurpose, InferenceConfig, OperationKind, WorkspaceIdentity};
use pkms_core::*;
use serde::Deserialize;
use std::{io::Read, path::PathBuf, time::Instant};
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
#[derive(Deserialize)]
struct Input {
    root: PathBuf,
    executable: PathBuf,
    endpoint: String,
    credential_identity: WorkspaceIdentity,
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::io::stdin().take(16 * 1024).read_to_end(&mut bytes)?;
    let input: Input = serde_json::from_slice(&bytes)?;
    if input.root.exists() {
        return Err("new directory required".into());
    }
    let old = CredentialBinding::new(
        &input.credential_identity.local_instance_id,
        &input.credential_identity.workspace_id,
        "openai_embeddings",
        &input.endpoint,
        pkms_inference::remote_chunks::MODEL,
    )?;
    let secret = vault::load(&old)?.ok_or("credential missing")?;
    let mut app = AppService::open(input.root.join("app"))?;
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, input.root.join("knowledge"))?;
    let accepted =
        app.accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)?;
    let identity = app
        .execute_operation(&accepted.operation_id)?
        .result
        .ok_or("create failed")?
        .identity;
    let binding = CredentialBinding::new(
        &identity.local_instance_id,
        &identity.workspace_id,
        "openai_embeddings",
        &input.endpoint,
        pkms_inference::remote_chunks::MODEL,
    )?;
    vault::store(&binding, &secret)?;
    let mut notes = Vec::new();
    for (title, text) in [
        (
            "合成蓝桥备份",
            "蓝桥知识库每天21:00备份，备份保留七天，副本存储在离线硬盘。",
        ),
        ("合成果园种植", "花园种了番茄和苹果，春天需要浇水施肥。"),
        (
            "合成Rust并发",
            "Rust使用所有权和借用检查来保证内存安全，线程共享状态可使用互斥锁。",
        ),
        ("合成天文观测", "望远镜用于观测银河中的恒星与行星。"),
    ] {
        notes.push(app.library_mut(&identity)?.create_note(title, text)?);
    }
    let long = "合成中文长资料😀，每次修改保留历史版本。".repeat(300);
    let long_note = app.library_mut(&identity)?.create_note("合成长文", &long)?;
    let db = app.library_mut(&identity)?;
    let job = db.accept_source_import(
        &id(),
        ImportSource {
            input_token: id(),
            original_name: "合成资料.txt".into(),
            format: SourceFormat::Text,
            origin_url: None,
            collection_id: None,
            add_version_to: None,
            base_row_version: None,
        },
    )?;
    let source_lease = db
        .claim_source_job(&job.job_id)?
        .ok_or("source lease missing")?;
    let text = "无关前文 授权备份证据：副本保留七天。 无关后文";
    let version = db.capture_source_original(
        &job.job_id,
        &source_lease.lease_token,
        &mut std::io::Cursor::new(text.as_bytes()),
        &mut |_| Ok(()),
    )?;
    db.publish_source_parse(
        &job.job_id,
        &source_lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text.as_bytes())?,
    )?;
    let block = db
        .source_text_page(&version.id, None, 0, 1)?
        .blocks
        .remove(0);
    let quote = "授权备份证据：副本保留七天。";
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: version.id.clone(),
                    block_id: block.id,
                    start_codepoint: 5,
                    end_codepoint: 5 + quote.chars().count() as u32,
                    quote: quote.into(),
                    prefix: "无关前文 ".into(),
                    suffix: " 无关后文".into(),
                },
                comment: String::new(),
            },
        )?
        .annotation
        .ok_or("annotation missing")?;
    let collection = db.create_collection("合成精准摘录主题")?;
    let base = db.collection_get(&collection.id)?.metadata.row_version;
    db.collection_add_members(
        &id(),
        ChangeCollectionMembers {
            collection_id: collection.id.clone(),
            base_row_version: base,
            member_ids: vec![annotation.id.clone()],
        },
    )?;
    let command = pkms_app::DesktopCommand::EmbeddingRemoteConfigure {
        command_id: id(),
        provider: EmbeddingProvider::Cloudflare,
        endpoint: input.endpoint,
        model: "@cf/baai/bge-m3".into(),
        output_dimensions: None,
    };
    app.dispatch(pkms_app::DesktopRequest {
        api_version: 1,
        request_id: id(),
        identity: Some(identity.clone()),
        command,
    })
    .map_err(|_| "embedding configuration failed")?;
    let config = InferenceConfig {
        executable: input.executable.canonicalize()?,
        model_directory: input.root.join("unused-models"),
        vector_root: input.root.join("vectors"),
        shutdown: Default::default(),
    };
    let started = Instant::now();
    app.inference_jobs_worker()?
        .ok_or("scheduler unavailable")?
        .run(config.clone(), &mut |_, _, _| {})?;
    let index = app.library(&identity)?.index_status()?;
    if index.ready_objects != 6 {
        return Err("index not complete".into());
    }
    let index_ms = started.elapsed().as_millis();
    let mut connection = app.inference_connection(&identity)?;
    let mut samples = Vec::new();
    for mode in [SearchMode::Semantic, SearchMode::Hybrid] {
        let q = SearchQuery {
            query: "什么时候做副本，保存几天？".into(),
            mode,
            scope: SearchScope::default(),
            limit: 5,
            minimum_cosine: None,
        };
        let started = Instant::now();
        let found = connection.search(&config, q.clone())?;
        std::fs::write(
            input.root.join(format!("query-{mode:?}.json")),
            serde_json::to_vec_pretty(&found)?,
        )?;
        if !found.semantic_available
            || found.items.first().is_none_or(|v| {
                v.target.object_id != notes[0].id && v.target.object_id != version.source_id
            })
            || !found
                .items
                .iter()
                .take(3)
                .any(|v| v.target.object_id == notes[0].id)
        {
            return Err("semantic ranking failed".into());
        }
        samples.push(serde_json::json!({"mode":mode,"ms":started.elapsed().as_secs_f64()*1000.,"semantic_available":found.semantic_available,"first":found.items[0].title}));
        let mut narrowed = q;
        narrowed.scope.object_ids = vec![notes[1].id.clone()];
        let result = connection.search(&config, narrowed)?;
        if result
            .items
            .iter()
            .any(|r| r.target.object_id != notes[1].id)
        {
            return Err("scope escaped".into());
        }
    }
    let precise = connection.search(
        &config,
        SearchQuery {
            query: "授权备份证据".into(),
            mode: SearchMode::Semantic,
            scope: SearchScope {
                collection_id: Some(collection.id),
                ..Default::default()
            },
            limit: 8,
            minimum_cosine: None,
        },
    )?;
    if !precise.semantic_available
        || precise.items.len() != 1
        || precise.items[0].text != quote
        || precise.items[0]
            .annotation
            .as_ref()
            .is_none_or(|a| a.annotation_id != annotation.id)
    {
        return Err("exact quote scope failed".into());
    }
    let metadata = app.library(&identity)?.object_metadata(&notes[0].id)?;
    app.library_mut(&identity)?.set_object_trashed(
        &id(),
        SetObjectFlag {
            object_id: notes[0].id.clone(),
            base_row_version: metadata.row_version,
            value: true,
        },
    )?;
    let found = connection.search(
        &config,
        SearchQuery {
            query: "备份".into(),
            mode: SearchMode::Hybrid,
            scope: SearchScope::default(),
            limit: 10,
            minimum_cosine: None,
        },
    )?;
    if found
        .items
        .iter()
        .any(|r| r.target.object_id == notes[0].id)
    {
        return Err("trashed result leaked".into());
    }
    let restored = app.library(&identity)?.note(&long_note.id)?;
    if restored.document.plain_text() != long {
        return Err("original changed".into());
    }
    let result = serde_json::json!({"identity":identity,"index_ms":index_ms,"ready_before_trash":index.ready_objects,"samples":samples,"scope_pass":true,"trash_pass":true,"original_unchanged":true,"exact_annotation_pass":true});
    std::fs::write(
        input.root.join("result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{result}");
    pkms_app::stop_inference_session();
    Ok(())
}

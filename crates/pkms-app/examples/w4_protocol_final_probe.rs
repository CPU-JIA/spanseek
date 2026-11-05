//! One explicit insufficient-evidence request per already-existing synthetic budget workspace.
//! No fixture initialization, embedding calls, UI operations or secret serialization.
use pkms_app::{
    AppService, DesktopCommand, DesktopData, DesktopRequest, GrantPurpose, InferenceConfig,
    OperationKind, WorkspaceAvailability, WorkspaceIdentity,
    inference_credentials::{self as vault, CredentialBinding},
};
use pkms_core::*;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

const QUESTION: &str = "松柏计划晨间批次和晚间批次备份保留期届满后，分别由哪位负责人批准销毁？请给出负责人姓名并引用记录。";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    budget_root: PathBuf,
    output_root: PathBuf,
    credential_identity: WorkspaceIdentity,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeCredentialInput {
    active_file: PathBuf,
    profile_evidence_file: PathBuf,
    report_file: PathBuf,
    credential_identity: WorkspaceIdentity,
}
fn remote_source_binding(
    identity: &WorkspaceIdentity,
) -> std::result::Result<CredentialBinding, Box<dyn std::error::Error>> {
    Ok(CredentialBinding::new(
        &identity.local_instance_id,
        &identity.workspace_id,
        "openai_chat",
        "https://api.deepseek.com",
        "deepseek-flash",
    )?)
}
fn native_credential_mode(
    mode: &str,
    bytes: &[u8],
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let input: NativeCredentialInput = serde_json::from_slice(bytes)?;
    if mode == "prepare-native" {
        if input.report_file.exists() {
            return Err(
                "native credential record already exists; do not overwrite ownership".into(),
            );
        }
        let active: Value = serde_json::from_slice(&std::fs::read(&input.active_file)?)?;
        let evidence: Value =
            serde_json::from_slice(&std::fs::read(&input.profile_evidence_file)?)?;
        let identity: WorkspaceIdentity =
            serde_json::from_value(active["active"]["identity"].clone())?;
        let profile: GenerationProfile =
            serde_json::from_value(evidence["request"]["model"].clone())?;
        if identity.workspace_id != evidence["identity"]["workspace_id"]
            || profile.provider != "anthropic"
            || profile.endpoint != "https://api.deepseek.com/anthropic/v1"
            || profile.model != "deepseek-flash"
        {
            return Err("native target or existing profile evidence mismatch".into());
        }
        let binding = CredentialBinding::new(
            &identity.local_instance_id,
            &identity.workspace_id,
            &profile.provider,
            &profile.endpoint,
            &profile.model,
        )?;
        let existed = vault::exists(&binding)?;
        let metadata = json!({"identity":identity,"provider":profile.provider,"endpoint":profile.endpoint,"model":profile.model});
        let metadata_hash = hash(&serde_json::to_vec(&metadata)?);
        let record = json!({"mode":"native_agent_temporary_binding","identity":identity,"profile":profile,"binding_metadata_sha256":metadata_hash,"existed_before":existed,"cleanup_required":!existed,"source_credential_identity":input.credential_identity});
        // Record ownership durably before any vault write. No key reaches this record.
        write_new(&input.report_file, &record)?;
        if !existed {
            let secret = vault::load(&remote_source_binding(&input.credential_identity)?)?
                .ok_or("source QA credential missing")?;
            vault::store(&binding, &secret)?;
        }
        if !vault::exists(&binding)? {
            return Err("native target binding not available after prepare".into());
        }
        println!(
            "{}",
            json!({"created":!existed,"identity":identity,"profile":profile,"binding_metadata_sha256":metadata_hash})
        );
        return Ok(());
    }
    let record: Value = serde_json::from_slice(&std::fs::read(&input.report_file)?)?;
    if record["mode"] != "native_agent_temporary_binding" {
        return Err("unexpected credential ownership record".into());
    }
    let identity: WorkspaceIdentity = serde_json::from_value(record["identity"].clone())?;
    let profile: GenerationProfile = serde_json::from_value(record["profile"].clone())?;
    let source_identity: WorkspaceIdentity =
        serde_json::from_value(record["source_credential_identity"].clone())?;
    let binding = CredentialBinding::new(
        &identity.local_instance_id,
        &identity.workspace_id,
        &profile.provider,
        &profile.endpoint,
        &profile.model,
    )?;
    let cleanup_required = record["cleanup_required"] == true && record["existed_before"] == false;
    let mut deleted = false;
    if cleanup_required && let Some(current) = vault::load(&binding)? {
        let source = vault::load(&remote_source_binding(&source_identity)?)?
            .ok_or("source changed; retain target for manual confirmation")?;
        if current.expose_secret() != source.expose_secret() {
            return Err("target credential changed; refusing to delete it".into());
        }
        vault::delete(&binding)?;
        deleted = true;
    }
    let exists_after = vault::exists(&binding)?;
    if cleanup_required && exists_after {
        return Err("temporary binding cleanup incomplete".into());
    }
    let outcome = json!({"deleted":deleted,"identity":identity,"cleanup_required":cleanup_required,"exists_after":exists_after,"binding_metadata_sha256":record["binding_metadata_sha256"]});
    write_new(&input.report_file.with_extension("cleanup.json"), &outcome)?;
    println!("{outcome}");
    Ok(())
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn write_new(path: &Path, value: &Value) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}
fn dispatch(
    app: &mut AppService,
    identity: &WorkspaceIdentity,
    command: DesktopCommand,
) -> std::result::Result<DesktopData, Box<dyn std::error::Error>> {
    app.dispatch(DesktopRequest {
        api_version: 1,
        request_id: id(),
        identity: Some(identity.clone()),
        command,
    })
    .map(|reply| reply.data)
    .map_err(|error| format!("App command failed: {:?}", error.code).into())
}
fn inventory(path: &Path) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut rows = db.prepare(
        "SELECT note_id,revision_id,content_hash FROM note_revisions ORDER BY note_id,revision_id",
    )?;
    // Include every immutable revision without copying any title or document text into logs.
    let revisions = rows.query_map([], |r| Ok(json!({"id":r.get::<_,String>(0)?,"revision_id":r.get::<_,String>(1)?,"content_hash":r.get::<_,String>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let mut counts = serde_json::Map::new();
    for table in [
        "notes",
        "note_revisions",
        "proposals",
        "answers",
        "index_rebuild_jobs",
    ] {
        let count: i64 =
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        counts.insert(table.into(), json!(count));
    }
    Ok(json!({"counts":counts,"revision_hashes":revisions}))
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::io::stdin().take(16384).read_to_end(&mut bytes)?;
    if let Some(mode) = std::env::args().nth(1) {
        if !matches!(mode.as_str(), "prepare-native" | "cleanup-native") {
            return Err("unknown mode".into());
        }
        return native_credential_mode(&mode, &bytes);
    }
    let input: Input = serde_json::from_slice(&bytes)?;
    if input.output_root.exists() {
        return Err("output already exists; never repeat a live probe".into());
    }
    let source = CredentialBinding::new(
        &input.credential_identity.local_instance_id,
        &input.credential_identity.workspace_id,
        "openai_chat",
        "https://api.deepseek.com",
        "deepseek-flash",
    )?;
    let secret =
        vault::load(&source)?.ok_or("existing QA vault binding absent; no live request sent")?;
    std::fs::create_dir(&input.output_root)?;
    let mut summaries = Vec::new();
    for (protocol, ids) in [
        (
            "openai_responses",
            [
                "c050219e-7364-44ab-9618-cfeab2413515",
                "18c3078f-4aac-4c72-babe-5c3b05ac4d33",
            ],
        ),
        (
            "anthropic",
            [
                "96beaa76-1850-4a94-bf68-0961ee2391f5",
                "7f1e3f10-7366-4174-a47c-e8c77d3f20d9",
            ],
        ),
    ] {
        let root = input.budget_root.join(protocol);
        if !root.join("knowledge/library.sqlite3").is_file()
            || !root.join("app/app-state.sqlite3").is_file()
        {
            return Err("existing budget workspace required".into());
        }
        let mut app = AppService::open(root.join("app"))?; // Acquires its exclusive application lock.
        let canonical = root.join("knowledge").canonicalize()?;
        let entry = app
            .workspaces()?
            .into_iter()
            .find(|entry| {
                Path::new(&entry.path) == canonical
                    && entry.availability == WorkspaceAvailability::Available
            })
            .ok_or("existing matching catalog entry unavailable")?;
        let grant = app.grant_path(GrantPurpose::OpenWorkspace, &canonical)?;
        let operation =
            app.accept_workspace_operation(&id(), OperationKind::OpenWorkspace, &grant.token)?;
        let actual = app
            .execute_operation(&operation.operation_id)?
            .result
            .ok_or("open failed")?
            .identity;
        if actual != entry.identity
            || app.active_workspace().as_ref().map(|e| &e.identity) != Some(&actual)
        {
            return Err("original local instance changed".into());
        }
        let library = app.library(&actual)?;
        if library.active_embedding_profile()?.is_some()
            || !library.pending_answer_ids(1)?.is_empty()
            || !library.pending_chat_run_ids(1)?.is_empty()
            || !library.pending_index_rebuild_ids(1)?.is_empty()
        {
            return Err(
                "workspace has unrelated pending inference or embeddings; no request sent".into(),
            );
        }
        let settings = library.generation_settings()?;
        let profile = settings
            .profile
            .as_ref()
            .ok_or("generation profile missing")?;
        let endpoint = if protocol == "anthropic" {
            "https://api.deepseek.com/anthropic/v1"
        } else {
            "https://api.deepseek.com"
        };
        if profile.provider != protocol
            || profile.endpoint != endpoint
            || profile.model != "deepseek-flash"
        {
            return Err("budget generation profile changed".into());
        }
        for note_id in ids {
            library.note(note_id)?;
        }
        let before = inventory(&canonical.join("library.sqlite3"))?;
        let binding = CredentialBinding::new(
            &actual.local_instance_id,
            &actual.workspace_id,
            protocol,
            endpoint,
            &profile.model,
        )?;
        let existed = vault::exists(&binding)?;
        if !existed {
            vault::store(&binding, &secret)?;
        }
        // Never delete or overwrite a binding that was already valid before this probe.
        let run = (|| -> std::result::Result<Value, Box<dyn std::error::Error>> {
            let query = SearchQuery {
                query: QUESTION.into(),
                mode: SearchMode::Lexical,
                scope: SearchScope {
                    object_ids: ids.iter().map(|id| (*id).into()).collect(),
                    ..Default::default()
                },
                limit: 8,
                minimum_cosine: None,
            };
            let DesktopData::SearchResults(found) = dispatch(
                &mut app,
                &actual,
                DesktopCommand::SearchQuery(query.clone()),
            )?
            else {
                return Err("wrong search reply".into());
            };
            if found.items.is_empty()
                || found
                    .items
                    .iter()
                    .any(|item| !ids.contains(&item.target.object_id.as_str()))
            {
                return Err("nonempty scoped evidence required; no model request sent".into());
            }
            let command = id();
            let accepted_path = input.output_root.join(format!("{protocol}-accepted.json"));
            write_new(
                &accepted_path,
                &json!({"identity":actual,"command_id":command,"protocol":protocol,"search":query,"max_output_tokens":8192,"binding_existed":existed,"before":before,"search_targets":found.items.iter().map(|e|json!({"evidence_id":e.id,"target":e.target,"text_sha256":hash(e.text.as_bytes())})).collect::<Vec<_>>()}),
            )?;
            let DesktopData::AnswerMutation(accepted) = dispatch(
                &mut app,
                &actual,
                DesktopCommand::AnswersStart {
                    command_id: command.clone(),
                    search: query,
                    max_output_tokens: Some(8192),
                },
            )?
            else {
                return Err("wrong acceptance reply".into());
            };
            let started = Instant::now();
            println!(
                "{protocol} accepted answer_id={} lexical_candidates={}",
                accepted.answer.id,
                found.items.len()
            );
            let config = InferenceConfig {
                executable: root.join("unused-runtime.exe"),
                model_directory: root.join("unused-models"),
                vector_root: root.join("unused-vectors"),
                shutdown: Default::default(),
            };
            let mut transitions = Vec::new();
            app.inference_jobs_worker()?.ok_or("worker unavailable")?.run(config, &mut |_, answer, _| { if let Some(answer) = answer { transitions.push(json!({"id":answer.id,"status":answer.status,"seq":answer.last_event_seq,"evidence_count":answer.evidence.len()})); } })?;
            let answer = app.library(&actual)?.answer(&accepted.answer.id)?;
            let after = inventory(&canonical.join("library.sqlite3"))?;
            let passed = answer.status == AnswerStatus::NoEvidence
                && answer.error_code.as_deref() == Some("insufficient_evidence")
                && !answer.evidence.is_empty();
            let unchanged = before["revision_hashes"] == after["revision_hashes"]
                && ["notes", "note_revisions", "proposals", "index_rebuild_jobs"]
                    .iter()
                    .all(|key| before["counts"][key] == after["counts"][key]);
            let row = json!({"protocol":protocol,"identity":actual,"command_id":command,"answer_id":answer.id,"status":answer.status,"error_code":answer.error_code,"last_event_seq":answer.last_event_seq,"request":answer.request,"evidence":answer.evidence.iter().map(|e|json!({"citation_id":e.citation_id,"evidence_id":e.evidence.id,"target":e.evidence.target,"text_sha256":hash(e.evidence.text.as_bytes())})).collect::<Vec<_>>(),"text_chars":answer.text.chars().count(),"text_sha256":hash(answer.text.as_bytes()),"cited_ids":answer.cited_ids,"transitions":transitions,"elapsed_ms":started.elapsed().as_millis(),"before":before,"after":after,"knowledge_unchanged":unchanged,"passed":passed});
            write_new(
                &input.output_root.join(format!("{protocol}-result.json")),
                &row,
            )?;
            println!(
                "{protocol} status={:?} evidence={} passed={passed} unchanged={unchanged}",
                answer.status,
                answer.evidence.len()
            );
            Ok(row)
        })();
        if !existed {
            vault::delete(&binding)?;
        }
        let preserved = vault::exists(&binding)? == existed;
        write_new(
            &input
                .output_root
                .join(format!("{protocol}-credential-cleanup.json")),
            &json!({"binding_existed_before":existed,"binding_state_preserved":preserved}),
        )?;
        summaries.push(run?);
        if !preserved {
            return Err("credential state differs from original".into());
        }
    }
    let passed = summaries
        .iter()
        .all(|row| row["passed"] == true && row["knowledge_unchanged"] == true);
    write_new(
        &input.output_root.join("summary.json"),
        &json!({"passed":passed,"results":summaries}),
    )?;
    if !passed {
        return Err("real result failed acceptance; do not retry or rewrite the question".into());
    }
    Ok(())
}

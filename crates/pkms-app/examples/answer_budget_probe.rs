//! Explicit synthetic live checks for frozen direct/Agent output budgets.
//! Reads only an existing QA vault binding; stdin/config/results never contain credentials.
use pkms_app::{
    AppService, GrantPurpose, InferenceConfig, OperationKind, WorkspaceIdentity,
    inference_credentials::{self as vault, CredentialBinding},
};
use pkms_core::*;
use serde::Deserialize;
use serde_json::json;
use std::{io::Read, path::PathBuf, time::Instant};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    root: PathBuf,
    credential_identity: WorkspaceIdentity,
    max_output_tokens: u32,
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
struct Cleanup(CredentialBinding);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = vault::delete(&self.0);
    }
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::io::stdin().take(16384).read_to_end(&mut bytes)?;
    let input: Input = serde_json::from_slice(&bytes)?;
    if input.root.exists() {
        return Err("probe root already exists".into());
    }
    let source = CredentialBinding::new(
        &input.credential_identity.local_instance_id,
        &input.credential_identity.workspace_id,
        "openai_chat",
        "https://api.deepseek.com",
        "deepseek-flash",
    )?;
    let secret = vault::load(&source)?.ok_or("existing synthetic credential required")?;
    let fixture: serde_json::Value =
        serde_json::from_slice(include_bytes!("../tests/fixtures/quality/development.json"))?;
    let question = fixture["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["id"].as_str() == Some("development/青竹版本/q4"))
        .ok_or("fixture query missing")?["question"]
        .as_str()
        .unwrap();
    let mut reports = Vec::new();
    for protocol in ["openai_chat", "openai_responses", "anthropic"] {
        let root = input.root.join(protocol);
        let mut app = AppService::open(root.join("app"))?;
        let grant = app.grant_path(GrantPurpose::CreateWorkspace, root.join("knowledge"))?;
        let accepted =
            app.accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)?;
        let identity = app
            .execute_operation(&accepted.operation_id)?
            .result
            .ok_or("create failed")?
            .identity;
        let endpoint = if protocol == "anthropic" {
            "https://api.deepseek.com/anthropic/v1"
        } else {
            "https://api.deepseek.com"
        };
        let profile = GenerationProfile {
            channel_id: None,
            capabilities: None,
            provider: protocol.into(),
            endpoint: endpoint.into(),
            model: "deepseek-flash".into(),
            model_digest: String::new(),
            config_revision: 1,
            temperature: 0.,
            context_tokens: 65536,
            chat_token_limit: if protocol == "openai_chat" {
                Some(ChatTokenLimit::MaxTokens)
            } else {
                None
            },
        };
        let binding = CredentialBinding::new(
            &identity.local_instance_id,
            &identity.workspace_id,
            protocol,
            endpoint,
            &profile.model,
        )?;
        vault::store(&binding, &secret)?;
        let _cleanup = Cleanup(binding);
        let db = app.library_mut(&identity)?;
        let mut direct_ids = Vec::new();
        for doc in fixture["documents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["id"].as_str().unwrap().contains("/青竹版本/"))
        {
            direct_ids.push(
                db.create_note(
                    doc["title"].as_str().unwrap(),
                    doc["text"].as_str().unwrap(),
                )?
                .id,
            );
        }
        let morning =
            db.create_note("松柏计划晨间批次", "松柏计划晨间批次的备份保留期限为7天。")?;
        let evening =
            db.create_note("松柏计划晚间批次", "松柏计划晚间批次的备份保留期限为13天。")?;
        db.set_generation_profile(
            &id(),
            SetGenerationProfile {
                base_row_version: 0,
                profile: Some(profile),
            },
        )?;
        let config = InferenceConfig {
            executable: root.join("unused-runtime.exe"),
            model_directory: root.join("unused-models"),
            vector_root: root.join("unused-vectors"),
            shutdown: Default::default(),
        };
        for agent in [false, true] {
            let question = if agent {
                "请使用知识库工具检索并读取松柏计划的晨间和晚间批次记录，分别回答两种批次的备份保留期限是多少天，并标注来源。"
            } else {
                question
            };
            let scope = SearchScope {
                object_ids: if agent {
                    vec![morning.id.clone(), evening.id.clone()]
                } else {
                    direct_ids.clone()
                },
                ..Default::default()
            };
            let search = SearchQuery {
                query: question.into(),
                mode: SearchMode::Lexical,
                scope,
                limit: 8,
                minimum_cosine: None,
            };
            let command_id = id();
            let command = if agent {
                pkms_app::DesktopCommand::AnswersAgentStart {
                    command_id: command_id.clone(),
                    search,
                    agent: AgentOptions::default(),
                    max_output_tokens: Some(input.max_output_tokens),
                }
            } else {
                pkms_app::DesktopCommand::AnswersStart {
                    command_id: command_id.clone(),
                    search,
                    max_output_tokens: Some(input.max_output_tokens),
                }
            };
            let request = pkms_app::DesktopRequest {
                api_version: 1,
                request_id: id(),
                identity: Some(identity.clone()),
                command,
            };
            let accepted = app
                .dispatch(request.clone())
                .map_err(|_| "answer acceptance failed")?;
            let pkms_app::DesktopData::AnswerMutation(accepted) = accepted.data else {
                return Err("wrong reply".into());
            };
            let started = Instant::now();
            let mut stages = Vec::new();
            app.inference_jobs_worker()?
                .ok_or("worker missing")?
                .run_with_progress(config.clone(), &mut |_, _, _| {}, &mut |_, stage| {
                    stages.push(stage.clone())
                })?;
            let answer = app.library(&identity)?.answer(&accepted.answer.id)?;
            let replay = app.dispatch(request).map_err(|_| "replay failed")?;
            let pkms_app::DesktopData::AnswerMutation(replay) = replay.data else {
                return Err("wrong replay".into());
            };
            assert!(replay.receipt.replayed);
            assert_eq!(replay.answer.id, answer.id);
            assert_eq!(
                answer.request.max_output_tokens,
                Some(input.max_output_tokens)
            );
            let kind = if agent { "agent" } else { "direct" };
            let row = json!({"protocol":protocol,"kind":kind,"elapsed_ms":started.elapsed().as_millis(),"identity":identity,"answer":answer,"stages":stages,"replay_same_id":true});
            std::fs::write(
                root.join(format!("{kind}.json")),
                serde_json::to_vec_pretty(&row)?,
            )?;
            println!(
                "{} {} status={:?} budget={} elapsed_ms={}",
                protocol,
                kind,
                answer.status,
                input.max_output_tokens,
                started.elapsed().as_millis()
            );
            reports.push(json!({"protocol":protocol,"kind":kind,"status":answer.status,"error":answer.error_code,"text":answer.text,"budget":input.max_output_tokens}));
        }
    }
    std::fs::write(
        input.root.join("summary.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    if reports.iter().any(|r| r["status"] != "completed") {
        return Err("one or more real requests did not complete; inspect evidence".into());
    }
    Ok(())
}

//! Synthetic interruption/restore fixture using production App/core APIs, no model calls.
use pkms_app::{AppService, GrantPurpose, OperationKind};
use pkms_core::*;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn start(db: &mut Library, c: &ChatConversation, text: &str) -> pkms_core::Result<ChatRunLease> {
    let accepted = db.accept_chat_run_from_settings(
        &id(),
        StartChatRun {
            conversation_id: c.id.clone(),
            base_row_version: c.row_version,
            text: text.into(),
            purpose: ChatPurpose::Chat,
            materials: vec![],
            scope: Default::default(),
            use_knowledge_tools: false,
            options: None,
        },
    )?;
    db.claim_chat_run(&accepted.run.id)?.ok_or(Error::NotFound)
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: chat_recovery_fixture <new-directory>")?,
    );
    if root.exists() {
        return Err("fixture destination must be new".into());
    }
    let mut app = AppService::open(root.join("app"))?;
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, root.join("knowledge"))?;
    let op = app.accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)?;
    let identity = app
        .execute_operation(&op.operation_id)?
        .result
        .ok_or("missing workspace")?
        .identity;
    let db = app.library_mut(&identity)?;
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 0,
            profile: Some(GenerationProfile {
                channel_id: None,
                capabilities: None,
                provider: "openai_chat".into(),
                endpoint: "https://example.invalid".into(),
                model: "synthetic-recovery-fixture".into(),
                model_digest: String::new(),
                config_revision: 1,
                temperature: 0.0,
                context_tokens: 65536,
                chat_token_limit: None,
            }),
        },
    )?;
    let mut c = db
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "合成状态矩阵 · 历史未完成回复".into(),
            },
        )?
        .conversation;
    let cases = [
        ("取消", ChatRunStatus::Canceled, None),
        (
            "中断",
            ChatRunStatus::Interrupted,
            Some("worker_interrupted"),
        ),
        ("超时", ChatRunStatus::TimedOut, Some("chat_total_timeout")),
        ("失败", ChatRunStatus::Failed, Some("model_unavailable")),
        ("截断", ChatRunStatus::Truncated, Some("chat_output_limit")),
        ("拒答", ChatRunStatus::Refused, None),
    ];
    for (label, status, error) in cases {
        let lease = start(db, &c, &format!("合成{label}测试"))?;
        db.append_chat_delta(
            &lease.run.id,
            &lease.lease_token,
            lease.run.last_event_seq + 1,
            &format!("**合成{label}片段**：保留这段未完成文字，不应因后续新回答而变为完整答案。"),
        )?;
        db.finish_chat_run(
            &lease.run.id,
            &lease.lease_token,
            status,
            error,
            ChatUsage::default(),
            None,
        )?;
        c = db.chat_conversation(&c.id)?;
    }
    for i in 1..=22 {
        let lease = start(db, &c, &format!("合成正常第{i}轮"))?;
        db.append_chat_delta(
            &lease.run.id,
            &lease.lease_token,
            lease.run.last_event_seq + 1,
            &format!("第{i}轮合成完整记录。用于核对前面历史状态不被较新回复覆盖。"),
        )?;
        db.finish_chat_run(
            &lease.run.id,
            &lease.lease_token,
            ChatRunStatus::Completed,
            None,
            ChatUsage::default(),
            Some("stop"),
        )?;
        c = db.chat_conversation(&c.id)?;
    }
    let running_c = db
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "合成运行中恢复 · 保留partial".into(),
            },
        )?
        .conversation;
    let lease = start(db, &running_c, "备份时仍在运行的合成请求")?;
    let partial = "运行中断前已持久保存的**合成片段**。\n不得自动重发。";
    let running = db.append_chat_delta(
        &lease.run.id,
        &lease.lease_token,
        lease.run.last_event_seq + 1,
        partial,
    )?;
    let queued_c = db
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "合成排队恢复 · 不自动重发".into(),
            },
        )?
        .conversation;
    let queued = db
        .accept_chat_run_from_settings(
            &id(),
            StartChatRun {
                conversation_id: queued_c.id.clone(),
                base_row_version: queued_c.row_version,
                text: "备份时尚未开始的合成请求".into(),
                purpose: ChatPurpose::Chat,
                materials: vec![],
                scope: Default::default(),
                use_knowledge_tools: false,
                options: None,
            },
        )?
        .run;
    let backup = db.create_backup(root.join("unfinished-chat.zip"), &mut |_| Ok(()))?;
    let report = json!({"synthetic":true,"identity":identity,"matrix_conversation":c.id,
        "running_conversation":running_c.id,"running_id":running.id,"running_seq":running.last_event_seq,"running_partial":partial,
        "queued_conversation":queued_c.id,"queued_id":queued.id,"queued_seq":queued.last_event_seq,"backup":backup});
    std::fs::write(
        root.join("fixture.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{}",
        json!({"directory":root,"conversations":3,"messages":60,"synthetic":true})
    );
    Ok(())
}

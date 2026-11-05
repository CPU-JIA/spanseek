//! Synthetic storage/rendering benchmark, not model-quality evidence. No network or credentials.
//! Writes only to a new, explicitly supplied QA directory.
use pkms_app::{AppService, GrantPurpose, OperationKind};
use pkms_core::*;
use serde_json::json;
use std::{path::PathBuf, time::Instant};
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let root =
        PathBuf::from(args.get(1).ok_or(
            "usage: chat_history_fixture <new-directory> [conversations=180] [rounds=500]",
        )?);
    let conversations: usize = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(180);
    let rounds: usize = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(500);
    if root.exists() || !(2..=10000).contains(&conversations) || !(1..=5000).contains(&rounds) {
        return Err("new directory and bounded positive counts required".into());
    }
    let mut app = AppService::open(root.join("app"))?;
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, root.join("knowledge"))?;
    let operation =
        app.accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)?;
    let identity = app
        .execute_operation(&operation.operation_id)?
        .result
        .ok_or("fixture creation failed")?
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
                model: "synthetic-history-fixture".into(),
                model_digest: String::new(),
                config_revision: 1,
                temperature: 0.0,
                context_tokens: 65536,
                chat_token_limit: None,
            }),
        },
    )?;
    let mut short_id = String::new();
    for i in 0..conversations - 1 {
        let c = db.create_chat_conversation(
            &id(),
            CreateConversation {
                title: format!("合成历史 {:04}", i + 1),
            },
        )?;
        short_id = c.conversation.id;
    }
    let mut conversation = db
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "合成长对话 · 分页性能验收".into(),
            },
        )?
        .conversation;
    let created = Instant::now();
    for i in 0..rounds {
        let accepted = db.accept_chat_run_from_settings(
            &id(),
            StartChatRun {
                conversation_id: conversation.id.clone(),
                base_row_version: conversation.row_version,
                text: format!("合成测试第 {:04} 轮：请整理记录与恢复的方法。", i + 1),
                purpose: ChatPurpose::Chat,
                materials: vec![],
                scope: Default::default(),
                use_knowledge_tools: false,
                options: None,
            },
        )?;
        let lease = db
            .claim_chat_run(&accepted.run.id)?
            .ok_or("missing fixture lease")?;
        let mut seq = lease.run.last_event_seq;
        for part in 1..=8 {
            let text = format!(
                "### 第 {:04} 轮 · 第 {part} 段\n\n这是**合成性能样本**，不是真实模型输出。资料需要核对来源，记录需要保留上下文。\n\n- 保存当前认识与原始证据。\n- 出现更正时保留历史版本。\n\n",
                i + 1
            );
            seq = db
                .append_chat_delta(&lease.run.id, &lease.lease_token, seq + 1, &text)?
                .last_event_seq;
        }
        db.finish_chat_run(
            &lease.run.id,
            &lease.lease_token,
            ChatRunStatus::Completed,
            None,
            ChatUsage::default(),
            Some("stop"),
        )?;
        conversation = db.chat_conversation(&conversation.id)?;
    }
    let seed_seconds = created.elapsed().as_secs_f64();
    let mut old_ms = vec![];
    let mut page_ms = vec![];
    let mut conversations_ms = vec![];
    let mut old_bytes = 0;
    let mut page_bytes = 0;
    for _ in 0..100 {
        let now = Instant::now();
        let mut all = vec![];
        let mut after = 0;
        loop {
            let items = db.chat_messages(&conversation.id, after, 100)?;
            let more = items.len() == 100;
            if let Some(last) = items.last() {
                after = last.ordinal;
            }
            all.extend(items);
            if !more {
                break;
            }
        }
        old_bytes = serde_json::to_vec(&all)?.len();
        old_ms.push(now.elapsed().as_secs_f64() * 1000.0);
        let now = Instant::now();
        let page = db.chat_messages_page(&conversation.id, None, 40)?;
        page_bytes = serde_json::to_vec(&page)?.len();
        page_ms.push(now.elapsed().as_secs_f64() * 1000.0);
        let now = Instant::now();
        let _ = serde_json::to_vec(&db.chat_conversations_page(None, 40)?)?;
        conversations_ms.push(now.elapsed().as_secs_f64() * 1000.0);
    }
    let report = json!({"synthetic":true,"profile":if cfg!(debug_assertions) {"debug"} else {"release"},
        "identity":identity,"conversation_id":conversation.id,"short_conversation_id":short_id,
        "conversations":conversations,"rounds":rounds,"messages":rounds*2,"seed_seconds":seed_seconds,
        "baseline_all_messages_bytes":old_bytes,"newest_page_bytes":page_bytes,
        "baseline_all_messages_ms":old_ms,"newest_page_ms":page_ms,"recent_conversations_ms":conversations_ms});
    std::fs::write(
        root.join("history-benchmark.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{}",
        json!({"directory":root,"conversations":conversations,"messages":rounds*2,"seed_seconds":seed_seconds})
    );
    Ok(())
}

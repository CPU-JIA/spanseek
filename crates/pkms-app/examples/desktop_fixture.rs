//! Isolated native smoke-test data. Uses the production application/core APIs;
//! no test transport or bypass exists in the desktop application.
use pkms_app::{AppService, GrantPurpose, OperationKind};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: desktop_fixture <new-test-directory>")?,
    );
    if root.exists() {
        return Err("fixture destination must be new".into());
    }
    let app_dir = root.join("app");
    let workspace = root.join("knowledge");
    let mut app = AppService::open(&app_dir)?;
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, &workspace)?;
    let operation = app.accept_workspace_operation(
        &uuid::Uuid::new_v4().to_string(),
        OperationKind::CreateWorkspace,
        &grant.token,
    )?;
    let created = app.execute_operation(&operation.operation_id)?;
    let identity = created.result.ok_or("fixture creation failed")?.identity;
    for (title, body) in [
        (
            "写给未来的自己",
            "知识不是收集的终点。\n留下此刻的理解，也留下日后修正的空间。",
        ),
        (
            "知识如何建立连接",
            "从一段阅读、一处疑问、一场对话开始。把值得保留的想法写下来，再将它们慢慢连接。",
        ),
        (
            "本周阅读清单",
            "读完一章后，试着用自己的话解释。哪些概念与已有经验相互印证？",
        ),
    ] {
        app.library_mut(&identity)?.create_note(title, body)?;
    }
    if let Some(image_path) = std::env::args().nth(2) {
        let path = PathBuf::from(image_path);
        let grant = app.grant_path(GrantPurpose::ImportFile, &path)?;
        let imported = app.import_selected_attachment(
            &identity,
            &uuid::Uuid::new_v4().to_string(),
            &grant.token,
        )?;
        let document = serde_json::from_value(serde_json::json!({"type":"doc","content":[
            {"type":"paragraph","attrs":{"id":uuid::Uuid::new_v4().to_string()},"content":[{"type":"text","text":"不可变图片与历史修订的真实读取验证。"}]},
            {"type":"image","attrs":{"id":uuid::Uuid::new_v4().to_string(),"blob_hash":imported.blob.hash,"original_name":imported.original_name,"alt":"桌面运行验证截图"}}
        ]}))?;
        app.library_mut(&identity)?.create_document(
            &uuid::Uuid::new_v4().to_string(),
            pkms_core::CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "图片与历史验证".into(),
                document_schema: 2,
                document,
            },
        )?;
    }
    let grant = app.grant_path(GrantPurpose::OpenWorkspace, &workspace)?;
    app.accept_workspace_operation(
        &uuid::Uuid::new_v4().to_string(),
        OperationKind::OpenWorkspace,
        &grant.token,
    )?;
    println!("{}", serde_json::to_string(&identity)?);
    Ok(())
}

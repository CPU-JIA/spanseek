//! Synthetic offline-export fixture, using the real core save/export commands.
use pkms_core::{CreateNote, ExportFormat, Library, NoteDocument};
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let destination = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("provide a new QA directory")?,
    );
    if destination.exists() {
        return Err("QA directory must not already exist".into());
    }
    std::fs::create_dir_all(&destination)?;
    let id = || Uuid::new_v4().to_string();
    let document: NoteDocument = serde_json::from_value(json!({"type":"doc","content":[
        {"type":"frontmatter","attrs":{"id":id(),"source":"fixture: offline-rich-export\n"}},
        {"type":"heading","attrs":{"id":id(),"level":6},"content":[{"type":"text","text":"富格式离线导出"}]},
        {"type":"paragraph","attrs":{"id":id()},"content":[
            {"type":"text","text":"行内公式："},
            {"type":"inlineMath","attrs":{"source":"E=mc^2"}},
            {"type":"text","text":"；彩色文字","marks":[{"type":"textStyle","attrs":{"color":"#2463EB"}},{"type":"underline"}]},
            {"type":"footnoteReference","attrs":{"label":"source"}}
        ]},
        {"type":"blockMath","attrs":{"id":id(),"source":"\\int_0^1 x^2\\,dx = \\frac{1}{3}"}},
        {"type":"codeBlock","attrs":{"id":id(),"language":"mermaid"},"content":[{"type":"text","text":"graph LR\n  A[本地正文] --> B[离线公式与图表]\n  B --> C[保留源码]"}]},
        {"type":"codeBlock","attrs":{"id":id(),"language":"mermaid"},"content":[{"type":"text","text":"graph LR\n  A -->"}]},
        {"type":"blockMath","attrs":{"id":id(),"source":"\\notARealCommand{source}"}},
        {"type":"commentBlock","attrs":{"id":id(),"source":"Private test comment <script>must never execute</script>"}},
        {"type":"footnoteDefinition","attrs":{"id":id(),"label":"source"},"content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"合成验证资料，不包含用户数据。"}]}]}
    ]}))?;
    let mut library = Library::open(destination.join("library"))?;
    let note = library
        .create_document(
            &id(),
            CreateNote {
                title: "富格式离线导出 <安全>".into(),
                document_schema: 3,
                document,
                collection_id: None,
                base_collection_row_version: None,
            },
        )?
        .note;
    for (format, filename) in [
        (ExportFormat::Html, "rich-html.zip"),
        (ExportFormat::Markdown, "rich-markdown.zip"),
    ] {
        let report = library.export_note_archive(
            &note.id,
            destination.join(filename),
            format,
            &mut |_| Ok(()),
        )?;
        println!("{}", serde_json::to_string(&report)?);
    }
    Ok(())
}

//! Reproducible synthetic scale data and warm core measurements. Real PDF worker;
//! no direct database writes, model output, or production transport bypasses.
use pkms_app::{AppService, GrantPurpose, OperationKind, SourceWorkerConfig};
use pkms_core::{
    CreateNote, ImportSource, Note, NotesPageRequest, SaveNote, SearchMode, SearchQuery,
    SearchScope, SourceFormat, SourcesPageRequest,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn summary(samples: &[Value]) -> Value {
    let mut times: Vec<f64> = samples.iter().map(|s| s["ms"].as_f64().unwrap()).collect();
    times.sort_by(f64::total_cmp);
    json!({"n":times.len(),"p50_ms":times[(times.len()*50).div_ceil(100)-1],"p95_ms":times[(times.len()*95).div_ceil(100)-1],"max_ms":times.last()})
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn pdf(index: usize) -> Vec<u8> {
    let streams: Vec<String> = (1..=2).map(|page| {
        let mut text=format!("BT /F1 12 Tf 50 780 Td (Synthetic PKMS source {index:03} page {page}) Tj 0 -24 Td ");
        for line in 0..18 { text.push_str(&format!("(Evidence record {line:02}: project topic{:02} review backup retrieval) Tj 0 -24 Td ",index%20)); }
        text.push_str("ET"); text
    }).collect();
    let objects=[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{}\nendstream",streams[0].len(),streams[0]),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{}\nendstream",streams[1].len(),streams[1]),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
    }
    let start = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        return Err("usage: workspace_performance_fixture <new-root> <notes>=1000 <pdfs>=100 <pkms-worker.exe> <pdfium.dll>".into());
    }
    let root = PathBuf::from(&args[1]);
    let count: usize = args[2].parse()?;
    let pdfs: usize = args[3].parse()?;
    if root.exists() || !(100..=10000).contains(&count) || !(1..=100).contains(&pdfs) {
        return Err("new directory, 100..10000 notes and 1..100 PDFs required".into());
    }
    let executable = fs::canonicalize(&args[4])?;
    let pdfium = fs::canonicalize(&args[5])?;
    let mut app = AppService::open(root.join("app"))?;
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, root.join("knowledge"))?;
    let op = app.accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)?;
    let identity = app
        .execute_operation(&op.operation_id)?
        .result
        .ok_or("workspace creation failed")?
        .identity;
    let started = Instant::now();
    let mut selected: Vec<Note> = Vec::new();
    let mut chars = 0usize;
    for index in 0..count {
        let content:Vec<Value>=(0..12).map(|block| {
            let text=format!("合成性能资料；不是用户内容。笔记编号 {index:05}，主题 topic{:02}，段落 {block:02}。记录阅读中的观点与证据，区分事实、推断和仍需查证的结论。知识管理需要反复检索、回顾和修订；原件保持不变，工作笔记保留版本。连接相关概念时应说明关系，应用新判断时应检查适用条件。Synthetic evidence supports project planning, review, reliable backup and search. unique{index:05} section{block:02}。",index%20);
            chars+=text.chars().count();
            json!({"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":text}]})
        }).collect();
        let note = app
            .library_mut(&identity)?
            .create_document(
                &id(),
                CreateNote {
                    title: format!("合成笔记 {index:05} · topic{:02}", index % 20),
                    document_schema: 2,
                    document: serde_json::from_value(json!({"type":"doc","content":content}))?,
                    collection_id: None,
                    base_collection_row_version: None,
                },
            )?
            .note;
        if index < 100 {
            selected.push(note);
        }
        if (index + 1) % 500 == 0 {
            eprintln!(
                "created {}/{count} notes ({:.1}s)",
                index + 1,
                started.elapsed().as_secs_f64()
            );
        }
    }
    let creation_ms = ms(started);
    fs::create_dir(root.join("inputs"))?;
    let mut source_bytes = 0usize;
    for index in 0..pdfs {
        let name = format!("synthetic-source-{index:03}.pdf");
        let file = root.join("inputs").join(&name);
        let bytes = pdf(index);
        source_bytes += bytes.len();
        fs::write(&file, bytes)?;
        let grant = app.grant_path(GrantPurpose::ImportSource, &file)?;
        app.import_selected_source(
            &identity,
            &id(),
            ImportSource {
                input_token: grant.token,
                original_name: name,
                format: SourceFormat::Pdf,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )?;
    }
    let started = Instant::now();
    app.source_jobs_worker()?
        .ok_or("no source worker")?
        .run(SourceWorkerConfig {
            executable,
            pdfium_library_path: Some(pdfium),
            ocr: None,
            shutdown: Default::default(),
        })?;
    let parsing_ms = ms(started);
    let sources = app.library(&identity)?.sources_page(SourcesPageRequest {
        filter: None,
        limit: 100,
        cursor: None,
        trashed: false,
        query: None,
    })?;
    if sources.items.len() != pdfs
        || sources
            .items
            .iter()
            .any(|s| !s.text_ready || !s.lexical_ready || s.format != SourceFormat::Pdf)
    {
        return Err("PDF import/parse did not complete".into());
    }
    let dataset = json!({"synthetic":true,"profile":if cfg!(debug_assertions){"debug"}else{"release"},"identity":identity,"notes":count,"note_characters":chars,"note_blocks":count*12,"sources":pdfs,"pdf_pages":pdfs*2,"original_bytes":source_bytes,"creation_ms":creation_ms,"pdf_import_parse_ms":parsing_ms,"sample_note_ids":selected.iter().map(|n|&n.id).collect::<Vec<_>>(),"source_ids":sources.items.iter().map(|s|&s.id).collect::<Vec<_>>()});
    write_json(&root.join("dataset.json"), &dataset)?;
    let mut samples = json!({"save":[],"keyword":[],"list":[]});
    let queries = [
        "知识管理",
        "topic03",
        "unique00007",
        "zxq987654novelterm",
        "reliable backup",
    ];
    for iteration in 0..110 {
        let note = &mut selected[iteration % 100];
        let mut document = note.document.clone();
        document.content.push(serde_json::from_value(json!({"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":format!("真实保存操作 {iteration:03}")}]}))?);
        let request = SaveNote {
            note_id: note.id.clone(),
            base_revision_id: note.revision_id.clone(),
            title: note.title.clone(),
            document_schema: 2,
            document,
        };
        let command = id();
        let started = Instant::now();
        let committed = app
            .library_mut(&identity)?
            .save_document(&command, request)?;
        let bytes = serde_json::to_vec(&committed)?.len();
        let elapsed = ms(started);
        *note = committed.note;
        if iteration >= 10 {
            samples["save"]
                .as_array_mut()
                .unwrap()
                .push(json!({"ms":elapsed,"json_bytes":bytes,"note_id":note.id}));
        }
        let query = queries[iteration % queries.len()];
        let request = SearchQuery {
            query: query.into(),
            mode: SearchMode::Lexical,
            scope: SearchScope::default(),
            limit: 20,
            minimum_cosine: None,
        };
        let started = Instant::now();
        let results = app.library(&identity)?.keyword_search(&request)?;
        let bytes = serde_json::to_vec(&results)?.len();
        let elapsed = ms(started);
        if query == queries[3] && !results.items.is_empty() {
            return Err("negative query returned unexpected evidence".into());
        }
        if query != queries[3] && results.items.is_empty() {
            return Err("known query lost evidence".into());
        }
        if iteration >= 10 {
            samples["keyword"].as_array_mut().unwrap().push(
                json!({"ms":elapsed,"query":query,"count":results.items.len(),"json_bytes":bytes}),
            );
        }
        let started = Instant::now();
        let page = app.library(&identity)?.notes_page(NotesPageRequest {
            limit: 40,
            query: None,
            cursor: None,
            trashed: false,
            filter: None,
            sort: None,
        })?;
        let bytes = serde_json::to_vec(&page)?.len();
        let elapsed = ms(started);
        if page.items.len() != 40 {
            return Err("incomplete first note page".into());
        }
        if iteration >= 10 {
            samples["list"]
                .as_array_mut()
                .unwrap()
                .push(json!({"ms":elapsed,"json_bytes":bytes}));
        }
    }
    let long_blocks = 1_500usize;
    let mut long_chars = 0usize;
    let long_content: Vec<Value> = (0..long_blocks)
        .map(|block| {
            let text = format!(
                "长文档性能样本；不是用户内容。段落 {block:04}。记录阅读中的观点与证据，区分事实、推断和仍需查证的结论。知识管理需要反复检索、回顾和修订；原件保持不变，工作笔记保留版本。连接相关概念时应说明关系，应用新判断时应检查适用条件。Synthetic long-document evidence supports project planning, review, reliable backup and search. Long section {block:04}."
            );
            long_chars += text.chars().count();
            json!({"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":text}]})
        })
        .collect();
    let started = Instant::now();
    let mut long_note = app
        .library_mut(&identity)?
        .create_document(
            &id(),
            CreateNote {
                title: "长文档性能样本".into(),
                document_schema: 2,
                document: serde_json::from_value(json!({"type":"doc","content":long_content}))?,
                collection_id: None,
                base_collection_row_version: None,
            },
        )?
        .note;
    let long_create_ms = ms(started);
    let mut long_get_samples: Vec<Value> = Vec::new();
    let mut long_save_samples: Vec<Value> = Vec::new();
    for iteration in 0..20 {
        let started = Instant::now();
        let note = app.library(&identity)?.note(&long_note.id)?.clone();
        let bytes = serde_json::to_vec(&note)?.len();
        let elapsed = ms(started);
        if iteration >= 5 {
            long_get_samples.push(json!({"ms":elapsed,"json_bytes":bytes}));
        }
        let mut document = note.document.clone();
        document.content.push(serde_json::from_value(json!({
            "type":"paragraph",
            "attrs":{"id":id()},
            "content":[{"type":"text","text":format!("追加段落 {iteration:02}")}]
        }))?);
        let command = id();
        let started = Instant::now();
        let committed = app.library_mut(&identity)?.save_document(
            &command,
            SaveNote {
                note_id: note.id.clone(),
                base_revision_id: note.revision_id.clone(),
                title: note.title.clone(),
                document_schema: 2,
                document,
            },
        )?;
        let bytes = serde_json::to_vec(&committed)?.len();
        let elapsed = ms(started);
        long_note = committed.note;
        if iteration >= 5 {
            long_save_samples.push(json!({"ms":elapsed,"json_bytes":bytes}));
        }
    }
    write_json(
        &root.join("long-document.json"),
        &json!({
            "note_id": long_note.id,
            "blocks": long_blocks,
            "characters": long_chars,
            "create_ms": long_create_ms,
            "get": summary(&long_get_samples),
            "save": summary(&long_save_samples),
        }),
    )?;
    write_json(&root.join("core-samples.json"), &samples)?;
    let result = json!({"measurement":"warm production core API + JSON serialization; no IPC or rendering; 10 warmups excluded; synchronous FULL unchanged","dataset":dataset,"save":summary(samples["save"].as_array().unwrap()),"keyword":summary(samples["keyword"].as_array().unwrap()),"list":summary(samples["list"].as_array().unwrap()),"long":{"blocks":long_blocks,"chars":long_chars,"create_ms":long_create_ms,"get":summary(&long_get_samples),"save":summary(&long_save_samples)}});
    write_json(&root.join("core-summary.json"), &result)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

//! Seeds an isolated, thematic workspace through public core commands.
//! Usage: knowledge_core_fixture <workspace> <fixture-root> <output-json>

use pkms_core::{
    AssignTag, CreateAnnotation, CreateNote, DocumentNode, DocumentType, ImportSource,
    KnowledgeObjectFilter, KnowledgeObjectPageRequest, Library, MetadataTarget, NodeAttributes,
    NodeType, Note, NoteDocument, ObjectKind, SourceFormat, SourceLocator, TagSelector,
    parse_text_source,
};
use serde_json::{Value, json};
use std::{fs, io::Cursor, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn text_node(value: &str) -> DocumentNode {
    DocumentNode {
        kind: NodeType::Text,
        attrs: NodeAttributes::default(),
        content: vec![],
        text: Some(value.to_owned()),
        marks: vec![],
    }
}

fn paragraph(content: Vec<DocumentNode>) -> DocumentNode {
    DocumentNode {
        kind: NodeType::Paragraph,
        attrs: NodeAttributes {
            id: id(),
            ..Default::default()
        },
        content,
        text: None,
        marks: vec![],
    }
}

fn reference_node(target: &Note) -> DocumentNode {
    DocumentNode {
        kind: NodeType::NoteReference,
        attrs: NodeAttributes {
            target_id: Some(target.id.clone()),
            target_revision_id: Some(target.revision_id.clone()),
            occurrence_id: Some(id()),
            label: Some(target.title.clone()),
            ..Default::default()
        },
        content: vec![],
        text: None,
        marks: vec![],
    }
}

fn document(body: &str, relation: Option<&Note>, inline: Option<&Note>) -> NoteDocument {
    let mut content = Vec::new();
    if let Some(target) = relation {
        content.push(DocumentNode {
            kind: NodeType::Frontmatter,
            attrs: NodeAttributes {
                id: id(),
                source: Some(format!(
                    "related:\n  - kind: note\n    id: {}\n    title: {}\n",
                    target.id, target.title
                )),
                ..Default::default()
            },
            content: vec![],
            text: None,
            marks: vec![],
        });
    }
    content.push(paragraph(vec![text_node(body)]));
    if let Some(target) = inline {
        content.push(paragraph(vec![
            text_node("正文引用："),
            reference_node(target),
            text_node("。该引用用于验证反链、图谱和画布共用同一关系真值。"),
        ]));
    }
    NoteDocument {
        kind: DocumentType::Doc,
        content,
    }
}

fn add_note(
    db: &mut Library,
    collection_id: &str,
    title: &str,
    body: &str,
    relation: Option<&Note>,
    inline: Option<&Note>,
) -> Result<Note> {
    let collection = db.collection_get(collection_id)?;
    Ok(db
        .create_document(
            &id(),
            CreateNote {
                title: title.to_owned(),
                document_schema: 3,
                document: document(body, relation, inline),
                collection_id: Some(collection_id.to_owned()),
                base_collection_row_version: Some(collection.metadata.row_version),
            },
        )?
        .note)
}

fn add_tags(db: &mut Library, object_id: &str, names: &[&str]) -> Result<()> {
    for name in names {
        let metadata = db.object_metadata(object_id)?;
        db.assign_tag(
            &id(),
            AssignTag {
                targets: vec![MetadataTarget {
                    object_id: object_id.to_owned(),
                    base_row_version: metadata.row_version,
                }],
                tag: TagSelector::Name {
                    name: (*name).to_owned(),
                },
            },
        )?;
    }
    Ok(())
}

fn import_text(
    db: &mut Library,
    root: &Path,
    relative: &str,
    collection_id: Option<String>,
) -> Result<(String, pkms_core::SourceVersion)> {
    let path = root.join(relative);
    let bytes = fs::read(&path)?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("fixture path has no file name")?
        .to_owned();
    let job = db.accept_source_import(
        &id(),
        ImportSource {
            input_token: id(),
            original_name: name,
            format: SourceFormat::Text,
            origin_url: None,
            collection_id,
            add_version_to: None,
            base_row_version: None,
        },
    )?;
    let lease = db
        .claim_source_job(&job.job_id)?
        .ok_or("source job was not claimable")?;
    let version = db.capture_source_original(
        &job.job_id,
        &lease.lease_token,
        &mut Cursor::new(bytes.as_slice()),
        &mut |_| Ok(()),
    )?;
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, bytes.as_slice())?,
    )?;
    Ok((version.source_id.clone(), version))
}

fn query(db: &Library, value: &str) -> Result<Value> {
    let page = db.knowledge_objects_page(KnowledgeObjectPageRequest {
        limit: 20,
        cursor: None,
        query: Some(value.to_owned()),
        filter: Some(KnowledgeObjectFilter {
            kinds: vec![ObjectKind::Note, ObjectKind::Source, ObjectKind::Annotation],
            ..Default::default()
        }),
    })?;
    Ok(json!({
        "query": value,
        "count": page.items.len(),
        "items": page.items.iter().map(|item| json!({
            "title": item.title,
            "kind": item.object.kind,
            "categories": item.collections.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    }))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        return Err(
            "usage: knowledge_core_fixture <workspace> <fixture-root> <output-json>".into(),
        );
    }
    let workspace = Path::new(&args[1]);
    let fixtures = Path::new(&args[2]);
    let output = Path::new(&args[3]);
    let mut db = Library::open(workspace)?;

    let embedded = db.create_collection("嵌入式开发")?;
    let research = db.create_collection("个人研究")?;
    let reading = db.create_collection("阅读与摘录")?;
    let retro = db.create_collection("项目复盘")?;
    let people = db.create_collection("人物与组织")?;
    let methods = db.create_collection("方法与工具")?;

    let embedded_note = add_note(
        &mut db,
        &embedded.id,
        "中断向量表与 RTOS 任务调度",
        "1 kHz 采样中断让 SPI 与 CAN 共享同一条处理路径。调试优先级反转时，先核对 NVIC 分组、临界区长度和任务切换点，再判断是不是总线物理层问题。",
        None,
        None,
    )?;
    add_tags(&mut db, &embedded_note.id, &["RTOS", "中断", "SPI"])?;

    let research_note = add_note(
        &mut db,
        &research.id,
        "访谈编码：研究问题如何收束",
        "第一轮开放式编码得到 18 个概念，第二轮只保留 6 个主题。反例必须保留，否则结论只是在重复自己的假设；证据链必须能回到原访谈。",
        None,
        None,
    )?;
    add_tags(&mut db, &research_note.id, &["访谈", "证据链", "研究方法"])?;

    let person_note = add_note(
        &mut db,
        &people.id,
        "人物与组织：林舟的访谈记录",
        "林舟负责控制板通信模块，主持了 10 月 21 日的故障复盘。他明确区分中断优先级问题和 CAN 终端电阻问题。",
        None,
        Some(&embedded_note),
    )?;
    add_tags(&mut db, &person_note.id, &["人物", "角色", "会议"])?;

    let method_note = add_note(
        &mut db,
        &methods.id,
        "方法与工具：检索策略与验证清单",
        "精确查询检查标题、笔记正文和资料原文块；模糊查询检查关键词、语义和分类 facet。语义索引不可用时必须明确回退，不把空结果写成通过。",
        Some(&research_note),
        None,
    )?;
    add_tags(&mut db, &method_note.id, &["检索", "验证", "备份"])?;

    let retro_note = add_note(
        &mut db,
        &retro.id,
        "项目复盘：控制板通信故障",
        "目标是把日志、访谈和修复决策放回同一个项目分类。重复导入不能制造无关系的新对象；导入时选分类即视为完成整理，不选则进入待整理。",
        Some(&method_note),
        None,
    )?;
    add_tags(&mut db, &retro_note.id, &["决策", "风险", "导入"])?;

    let can_note = add_note(
        &mut db,
        &embedded.id,
        "SPI 时序与 CAN 总线故障排查笔记",
        "SPI 先核对 CPOL/CPHA 与 DMA 完成中断；CAN 再检查 120Ω 终端电阻、错误帧计数和总线负载。连续运行 6 小时无错误后才结束复盘。",
        Some(&embedded_note),
        Some(&embedded_note),
    )?;
    add_tags(&mut db, &can_note.id, &["CAN", "SPI", "故障排查"])?;

    let (embedded_source, embedded_version) = import_text(
        &mut db,
        fixtures,
        "嵌入式开发/SPI时序与CAN总线故障排查.md",
        Some(embedded.id.clone()),
    )?;
    add_tags(&mut db, &embedded_source, &["CAN", "SPI", "中断"])?;

    let (research_source, _) = import_text(
        &mut db,
        fixtures,
        "个人研究/访谈提纲-v3.md",
        Some(research.id.clone()),
    )?;
    add_tags(&mut db, &research_source, &["访谈", "研究方法"])?;

    let (reading_source, reading_version) = import_text(
        &mut db,
        fixtures,
        "阅读与摘录/设计数据密集型应用-第128页.md",
        Some(reading.id.clone()),
    )?;
    add_tags(&mut db, &reading_source, &["页码", "摘录", "一致性"])?;
    let block = db
        .source_text_page(&reading_version.id, None, 0, 20)?
        .blocks
        .into_iter()
        .find(|block| block.text.contains("复制延迟不是异常"))
        .ok_or("excerpt block was not parsed")?;
    let quote = block.text.clone();
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: reading_version.id.clone(),
                    block_id: block.id,
                    start_codepoint: 0,
                    end_codepoint: quote.chars().count() as u32,
                    quote,
                    prefix: String::new(),
                    suffix: String::new(),
                },
                comment: "复制延迟不是异常，而是系统设计必须暴露给使用者的状态。".into(),
            },
        )?
        .annotation
        .ok_or("annotation was not returned")?;
    add_tags(&mut db, &annotation.id, &["摘录", "复制"])?;

    let (inbox_source, _) = import_text(&mut db, fixtures, "项目复盘/控制板通信故障复盘.md", None)?;
    add_tags(&mut db, &inbox_source, &["待整理", "项目复盘"])?;

    let queries = [
        "中断向量表",
        "反例必须保留",
        "120Ω 终端电阻",
        "复制延迟不是异常",
        "导入时选分类即视为完成整理",
    ];
    let mut query_results = Vec::new();
    for value in queries {
        query_results.push(query(&db, value)?);
    }

    let result = json!({
        "synthetic": true,
        "workspace": workspace,
        "categories": {
            "嵌入式开发": embedded.id,
            "个人研究": research.id,
            "阅读与摘录": reading.id,
            "项目复盘": retro.id,
            "人物与组织": people.id,
            "方法与工具": methods.id,
        },
        "notes": {
            "embedded": embedded_note.id,
            "research": research_note.id,
            "person": person_note.id,
            "method": method_note.id,
            "retro": retro_note.id,
            "can": can_note.id,
        },
        "sources": {
            "embedded": embedded_source,
            "embedded_version": embedded_version.id,
            "research": research_source,
            "reading": reading_source,
            "reading_version": reading_version.id,
            "inbox": inbox_source,
        },
        "annotation": annotation.id,
        "queries": query_results,
    });
    fs::write(output, serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

//! Seeds a self-contained, realistic desktop demo library through public core APIs.
//! Usage: desktop_demo_library <workspace> <fixture-root> <report-json> <backup-path>
use chrono::{Duration, Utc};
use pkms_core::{
    AddReview, ApplyChangeItem, AssignTag, BoardGroupBy, CanvasNode, ChangePayload, ChangeSetKind,
    CollectionDetail, CreateAnnotation, CreateChangeSet, CreateCollection, CreateNote,
    CreateSourceNote, ImportSource, InboxState, KnowledgeObjectFilter, KnowledgeObjectPageRequest,
    Library, MetadataTarget, Note, NoteDocument, NoteSort, NotesFilter, ObjectKind, PdfReadingView,
    PdfZoomMode, ReadingStatus, ResolveLink, SaveCanvas, SaveNote, SaveView, SetInboxState,
    SetObjectFlag, SetReadingState, SetReviewInsightSettings, SnoozeReview, SourceFormat,
    SourceLocator, SourceSummary, SourceVersion, TagSelector, ViewConfig, ViewLayout, ViewProperty,
};
use pkms_worker::{WorkerRequest, execute, read_validated_result};
use serde_json::{Map, Value, json};
use std::{fs, io::Cursor, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn text(value: &str) -> Value {
    json!({"type":"text","text":value})
}

fn paragraph(value: &str) -> Value {
    paragraph_with_id(&id(), value)
}

fn paragraph_with_id(block_id: &str, value: &str) -> Value {
    json!({
        "type": "paragraph",
        "attrs": {"id": block_id},
        "content": [text(value)]
    })
}

fn heading(level: u8, value: &str) -> Value {
    json!({
        "type": "heading",
        "attrs": {"id": id(), "level": level},
        "content": [text(value)]
    })
}

fn bullet_list(items: &[&str]) -> Value {
    let entries = items
        .iter()
        .map(|item| {
            json!({
                "type": "listItem",
                "attrs": {"id": id()},
                "content": [paragraph(item)]
            })
        })
        .collect::<Vec<_>>();
    json!({
        "type": "bulletList",
        "attrs": {"id": id()},
        "content": entries
    })
}

fn task_list(items: &[(&str, bool)]) -> Value {
    let entries = items
        .iter()
        .map(|(item, checked)| {
            json!({
                "type": "taskItem",
                "attrs": {"id": id(), "checked": checked},
                "content": [paragraph(item)]
            })
        })
        .collect::<Vec<_>>();
    json!({
        "type": "taskList",
        "attrs": {"id": id()},
        "content": entries
    })
}

fn blockquote(value: &str) -> Value {
    json!({
        "type": "blockquote",
        "attrs": {"id": id()},
        "content": [paragraph(value)]
    })
}

fn code_block(language: &str, value: &str) -> Value {
    json!({
        "type": "codeBlock",
        "attrs": {"id": id(), "language": language},
        "content": [{"type": "text", "text": value}]
    })
}

fn callout(kind: &str, title: &str, value: &str) -> Value {
    json!({
        "type": "callout",
        "attrs": {"id": id(), "callout_type": kind, "title": title, "collapsed": false},
        "content": [paragraph(value)]
    })
}

fn reference_inline(target: &Note, target_block_id: Option<&str>) -> Value {
    let mut attrs = json!({
        "target_id": target.id,
        "target_revision_id": target.revision_id,
        "occurrence_id": id(),
        "label": target.title
    });
    if let Some(block_id) = target_block_id {
        attrs["target_block_id"] = json!(block_id);
    }
    json!({
        "type": "noteReference",
        "attrs": attrs
    })
}

fn reference_paragraph(prefix: &str, target: &Note, block_id: Option<&str>, suffix: &str) -> Value {
    json!({
        "type": "paragraph",
        "attrs": {"id": id()},
        "content": [text(prefix), reference_inline(target, block_id), text(suffix)]
    })
}

fn frontmatter(kind: &str, date: &str, extras: &[(&str, &str)], relations: &[&Note]) -> Value {
    let mut lines = vec![format!("type: {kind}"), format!("date: {date}")];
    for (key, value) in extras {
        lines.push(format!("{key}: {value}"));
    }
    if !relations.is_empty() {
        lines.push("related:".to_owned());
        for target in relations {
            lines.extend([
                "  - kind: note".to_owned(),
                format!("    id: {}", target.id),
                format!("    title: \"{}\"", target.title),
            ]);
        }
    }
    json!({
        "type": "frontmatter",
        "attrs": {"id": id(), "source": lines.join("\n") + "\n"}
    })
}

fn document(children: Vec<Value>) -> NoteDocument {
    serde_json::from_value(json!({"type": "doc", "content": children}))
        .expect("demo note document must use the supported editor schema")
}

fn collection(db: &Library, id: &str) -> Result<CollectionDetail> {
    Ok(db.collection_get(id)?)
}

fn create_note(
    db: &mut Library,
    collection_id: Option<&str>,
    title: &str,
    children: Vec<Value>,
) -> Result<Note> {
    let (collection_id, base_collection_row_version) = match collection_id {
        Some(value) => (
            Some(value.to_owned()),
            Some(collection(db, value)?.metadata.row_version),
        ),
        None => (None, None),
    };
    Ok(db
        .create_document(
            &id(),
            CreateNote {
                title: title.to_owned(),
                document_schema: 3,
                document: document(children),
                collection_id,
                base_collection_row_version,
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

fn add_members(db: &mut Library, collection_id: &str, member_ids: &[&str]) -> Result<()> {
    if member_ids.is_empty() {
        return Ok(());
    }
    let current = collection(db, collection_id)?;
    db.collection_add_members(
        &id(),
        pkms_core::ChangeCollectionMembers {
            collection_id: collection_id.to_owned(),
            base_row_version: current.metadata.row_version,
            member_ids: member_ids.iter().map(|value| (*value).to_owned()).collect(),
        },
    )?;
    Ok(())
}

fn set_inbox(db: &mut Library, object_id: &str, state: InboxState) -> Result<()> {
    let metadata = db.object_metadata(object_id)?;
    db.set_inbox_state(
        &id(),
        SetInboxState {
            object_id: object_id.to_owned(),
            base_row_version: metadata.row_version,
            target: state,
        },
    )?;
    Ok(())
}

fn trash_object(db: &mut Library, object_id: &str) -> Result<()> {
    let metadata = db.object_metadata(object_id)?;
    db.set_object_trashed(
        &id(),
        SetObjectFlag {
            object_id: object_id.to_owned(),
            base_row_version: metadata.row_version,
            value: true,
        },
    )?;
    Ok(())
}

fn import_file(
    db: &mut Library,
    path: &Path,
    output_root: &Path,
    format: SourceFormat,
    collection_id: Option<&str>,
    origin_url: Option<&str>,
    pdfium: Option<&Path>,
) -> Result<(SourceSummary, SourceVersion, pkms_core::ParsedSource)> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("fixture path must have a UTF-8 file name")?;
    let bytes = fs::read(path)?;
    let job = db.accept_source_import(
        &id(),
        ImportSource {
            input_token: id(),
            original_name: name.to_owned(),
            format,
            origin_url: origin_url.map(str::to_owned),
            collection_id: collection_id.map(str::to_owned),
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
    let output_dir = output_root.join(format!("parse-{}", version.id));
    fs::create_dir_all(&output_dir)?;
    let request = WorkerRequest {
        protocol_version: pkms_worker::PROTOCOL_VERSION,
        job_id: job.job_id.clone(),
        attempt: lease.job.attempt,
        lease_token: lease.lease_token.clone(),
        source_version_id: version.id.clone(),
        input_path: path.canonicalize()?,
        format,
        output_dir: output_dir.canonicalize()?,
        pdfium_library_path: pdfium.map(Path::to_path_buf),
        content_type: None,
        ocr: None,
    };
    let response = execute(&request);
    let parsed = read_validated_result(&request, &response).map_err(|error| {
        std::io::Error::other(format!("worker parse failed for {name}: {error}"))
    })?;
    db.publish_source_parse(&job.job_id, &lease.lease_token, parsed.clone())?;
    let source = db.source(&version.source_id)?;
    Ok((source, version, parsed))
}

fn find_text_locator(
    db: &Library,
    version: &SourceVersion,
    candidates: &[&str],
) -> Result<(SourceLocator, Option<u32>)> {
    let mut offset = 0_u32;
    loop {
        let page = db.source_text_page(&version.id, None, offset, 100)?;
        for block in &page.blocks {
            for keyword in candidates {
                if let Some(byte_start) = block.text.find(keyword) {
                    let start = block.text[..byte_start].chars().count() as u32;
                    let end = start + keyword.chars().count() as u32;
                    let chars = block.text.chars().collect::<Vec<_>>();
                    let prefix = chars[start.saturating_sub(24) as usize..start as usize]
                        .iter()
                        .collect::<String>();
                    let suffix_end = (end as usize + 24).min(chars.len());
                    let suffix = chars[end as usize..suffix_end].iter().collect::<String>();
                    return Ok((
                        SourceLocator::Text {
                            source_version_id: version.id.clone(),
                            block_id: block.id.clone(),
                            start_codepoint: start,
                            end_codepoint: end,
                            quote: (*keyword).to_owned(),
                            prefix,
                            suffix,
                        },
                        block.page_index,
                    ));
                }
            }
        }
        match page.next_offset {
            Some(next) if next > offset => offset = next,
            _ => break,
        }
    }
    Err(format!("annotation keyword not found in {}", version.original_name).into())
}

fn find_pdf_locator(
    db: &Library,
    version: &SourceVersion,
    candidates: &[&str],
) -> Result<(SourceLocator, u32)> {
    let mut offset = 0_u32;
    loop {
        let page = db.source_text_page(&version.id, None, offset, 100)?;
        for block in &page.blocks {
            let Some(geometry) = &block.pdf_geometry else {
                continue;
            };
            for keyword in candidates {
                let Some(byte_start) = block.text.find(keyword) else {
                    continue;
                };
                let start = block.text[..byte_start].chars().count() as u32;
                let end = start + keyword.chars().count() as u32;
                let chars = block.text.chars().collect::<Vec<_>>();
                let prefix = chars[start.saturating_sub(24) as usize..start as usize]
                    .iter()
                    .collect::<String>();
                let suffix_end = (end as usize + 24).min(chars.len());
                let suffix = chars[end as usize..suffix_end].iter().collect::<String>();
                let rects = geometry
                    .chars
                    .iter()
                    .filter(|item| item.end_codepoint > start && item.start_codepoint < end)
                    .filter_map(|item| item.rect)
                    .collect::<Vec<_>>();
                if rects.is_empty() {
                    continue;
                }
                let page_index = block.page_index.ok_or("PDF block lacks page index")?;
                return Ok((
                    SourceLocator::Pdf {
                        source_version_id: version.id.clone(),
                        page_index,
                        page_label: geometry.page_label.clone(),
                        crop_box: geometry.crop_box,
                        rotation: geometry.rotation,
                        coordinate_space: geometry.coordinate_space.clone(),
                        rects,
                        quote: (*keyword).to_owned(),
                        prefix,
                        suffix,
                    },
                    page_index,
                ));
            }
        }
        match page.next_offset {
            Some(next) if next > offset => offset = next,
            _ => break,
        }
    }
    Err(format!(
        "PDF annotation keyword not found in {}",
        version.original_name
    )
    .into())
}

fn create_annotation_for(
    db: &mut Library,
    version: &SourceVersion,
    is_pdf: bool,
    candidates: &[&str],
    comment: &str,
) -> Result<(pkms_core::Annotation, Option<u32>)> {
    let (locator, page_index) = if is_pdf {
        let (locator, page) = find_pdf_locator(db, version, candidates)?;
        (locator, Some(page))
    } else {
        find_text_locator(db, version, candidates)?
    };
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator,
                comment: comment.to_owned(),
            },
        )?
        .annotation
        .ok_or("annotation was not returned")?;
    Ok((annotation, page_index))
}

fn add_review(db: &mut Library, target_id: &str) -> Result<pkms_core::ReviewEntry> {
    Ok(db
        .add_review(
            &id(),
            AddReview {
                target: ResolveLink {
                    target_id: target_id.to_owned(),
                    target_block_id: None,
                    target_revision_id: None,
                },
                base_row_version: None,
            },
        )?
        .entry)
}

fn knowledge_query(db: &Library, query: &str) -> Result<Value> {
    let page = db.knowledge_objects_page(KnowledgeObjectPageRequest {
        limit: 20,
        cursor: None,
        query: Some(query.to_owned()),
        filter: Some(KnowledgeObjectFilter {
            kinds: vec![ObjectKind::Note, ObjectKind::Source, ObjectKind::Annotation],
            ..Default::default()
        }),
    })?;
    Ok(json!({
        "query": query,
        "count": page.items.len(),
        "titles": page.items.iter().map(|item| item.title.clone()).collect::<Vec<_>>()
    }))
}

fn object_types_registry(workspace_id: &str) -> Result<String> {
    let types = json!([
        {
            "id": "fault-case",
            "labelZh": "故障案例",
            "labelEn": "Fault case",
            "pluralZh": "故障案例",
            "pluralEn": "Fault cases",
            "descriptionZh": "记录一个可复现的工程故障、证据链、根因与修复结果。",
            "descriptionEn": "A reproducible engineering fault with evidence, cause, and repair.",
            "icon": "custom",
            "tone": "amber",
            "aliases": ["故障案例", "faultcase", "case-study"],
            "fields": [
                {"key":"summary","labelZh":"摘要","labelEn":"Summary","kind":"text","placeholderZh":"一句话说明故障与结果","placeholderEn":"One-line fault summary"},
                {"key":"severity","labelZh":"严重度","labelEn":"Severity","kind":"select","options":[{"value":"high","zh":"高","en":"High"},{"value":"medium","zh":"中","en":"Medium"},{"value":"low","zh":"低","en":"Low"}]},
                {"key":"review_phase","labelZh":"复盘阶段","labelEn":"Review phase","kind":"select","options":[{"value":"observed","zh":"观察","en":"Observed"},{"value":"root-cause","zh":"根因确认","en":"Root cause"},{"value":"verified","zh":"已验证","en":"Verified"}]},
                {"key":"occurred_on","labelZh":"发生日期","labelEn":"Occurred on","kind":"date"},
                {"key":"related","labelZh":"相关记录","labelEn":"Related records","kind":"relation"}
            ],
            "sections": [{"zh":"现象与影响","en":"Symptom and impact"},{"zh":"证据链","en":"Evidence"},{"zh":"根因与修复","en":"Cause and fix"},{"zh":"验证与遗留风险","en":"Verification and risk"}]
        },
        {
            "id": "experiment-log",
            "labelZh": "实验记录",
            "labelEn": "Experiment log",
            "pluralZh": "实验记录",
            "pluralEn": "Experiment logs",
            "descriptionZh": "保存测量条件、观察点、阈值与可重复的实验结论。",
            "descriptionEn": "Measurement conditions, observations, thresholds, and reproducible conclusions.",
            "icon": "custom",
            "tone": "teal",
            "aliases": ["实验记录", "experiment", "measurement-log"],
            "fields": [
                {"key":"summary","labelZh":"摘要","labelEn":"Summary","kind":"text"},
                {"key":"test_window","labelZh":"测试窗口","labelEn":"Test window","kind":"text"},
                {"key":"result","labelZh":"结果","labelEn":"Result","kind":"select","options":[{"value":"pass","zh":"通过","en":"Pass"},{"value":"fail","zh":"失败","en":"Fail"},{"value":"partial","zh":"部分通过","en":"Partial"}]},
                {"key":"measured_at","labelZh":"测量日期","labelEn":"Measured at","kind":"date"},
                {"key":"related","labelZh":"相关记录","labelEn":"Related records","kind":"relation"}
            ],
            "sections": [{"zh":"条件","en":"Conditions"},{"zh":"观察点","en":"Observations"},{"zh":"判定","en":"Verdict"},{"zh":"异常与解释","en":"Exceptions"}]
        },
        {
            "id": "review-meeting",
            "labelZh": "评审会议",
            "labelEn": "Review meeting",
            "pluralZh": "评审会议",
            "pluralEn": "Review meetings",
            "descriptionZh": "沉淀参会人、结论、决策、责任人与截止时间。",
            "descriptionEn": "Participants, conclusions, decisions, owners, and deadlines.",
            "icon": "custom",
            "tone": "violet",
            "aliases": ["评审会", "review-meeting-note", "decision-meeting"],
            "fields": [
                {"key":"summary","labelZh":"摘要","labelEn":"Summary","kind":"text"},
                {"key":"meeting_date","labelZh":"会议日期","labelEn":"Meeting date","kind":"date"},
                {"key":"participants","labelZh":"参会人","labelEn":"Participants","kind":"text"},
                {"key":"decision_status","labelZh":"决策状态","labelEn":"Decision status","kind":"select","options":[{"value":"proposed","zh":"待确认","en":"Proposed"},{"value":"accepted","zh":"已通过","en":"Accepted"},{"value":"blocked","zh":"有保留","en":"Blocked"}]},
                {"key":"related","labelZh":"相关记录","labelEn":"Related records","kind":"relation"}
            ],
            "sections": [{"zh":"议题","en":"Agenda"},{"zh":"结论","en":"Conclusion"},{"zh":"行动项","en":"Action items"},{"zh":"保留意见","en":"Reservations"}]
        }
    ]);
    let mut workspaces = Map::new();
    workspaces.insert(workspace_id.to_owned(), types);
    Ok(json!({"schema": 1, "workspaces": workspaces}).to_string())
}

fn note_templates_registry(workspace_id: &str) -> Result<String> {
    let templates = json!([
        {
            "id": "fault-case-review",
            "nameZh": "故障案例复盘",
            "nameEn": "Fault case review",
            "descriptionZh": "按现象、证据、根因、修复和验证顺序创建故障案例。",
            "descriptionEn": "Create a fault case with symptoms, evidence, cause, fix, and verification.",
            "typeId": "fault-case",
            "titleZh": "故障案例：{{date}}",
            "titleEn": "Fault case: {{date}}",
            "bodyZh": "## 现象与影响\n\n## 时间基准与复现条件\n\n## 证据链\n\n- \n\n## 根因\n\n## 修复与验证\n\n## 遗留风险\n",
            "bodyEn": "## Symptom and impact\n\n## Time base and reproduction\n\n## Evidence\n\n- \n\n## Root cause\n\n## Fix and verification\n\n## Remaining risk\n"
        },
        {
            "id": "experiment-log",
            "nameZh": "实验记录",
            "nameEn": "Experiment log",
            "descriptionZh": "记录条件、观察点、测量值和判定线。",
            "descriptionEn": "Record conditions, observation points, measurements, and thresholds.",
            "typeId": "experiment-log",
            "titleZh": "实验记录：{{date}}",
            "titleEn": "Experiment log: {{date}}",
            "bodyZh": "## 条件\n\n- 固件：\n- 设备：\n- 环境：\n\n## 观察点\n\n1. \n\n## 判定\n\n## 异常与解释\n",
            "bodyEn": "## Conditions\n\n- Firmware:\n- Device:\n- Environment:\n\n## Observations\n\n1. \n\n## Verdict\n\n## Exceptions\n"
        },
        {
            "id": "review-meeting-minute",
            "nameZh": "评审会议纪要",
            "nameEn": "Review meeting minutes",
            "descriptionZh": "记录参会人、证据复核、决策、行动项与保留意见。",
            "descriptionEn": "Capture participants, evidence review, decisions, actions, and reservations.",
            "typeId": "review-meeting",
            "titleZh": "会议纪要：{{date}}",
            "titleEn": "Meeting minutes: {{date}}",
            "bodyZh": "## 参会人\n\n## 证据复核\n\n## 决策\n\n1. \n\n## 行动项\n\n- 负责人：\n- 截止时间：\n\n## 保留意见\n",
            "bodyEn": "## Participants\n\n## Evidence review\n\n## Decisions\n\n1. \n\n## Action items\n\n- Owner:\n- Due:\n\n## Reservations\n"
        }
    ]);
    let mut workspaces = Map::new();
    workspaces.insert(workspace_id.to_owned(), templates);
    Ok(json!({"schema": 1, "workspaces": workspaces}).to_string())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err(
            "usage: desktop_demo_library <workspace> <fixture-root> <report-json> <backup-path>"
                .into(),
        );
    }
    let workspace = Path::new(&args[1]);
    if workspace.exists() {
        return Err(format!("workspace already exists: {}", workspace.display()).into());
    }
    fs::create_dir_all(workspace)?;
    let fixtures = Path::new(&args[2]).canonicalize()?;
    let report_path = Path::new(&args[3]);
    let backup_path = Path::new(&args[4]);
    let output_root = report_path
        .parent()
        .ok_or("report path has no parent")?
        .join("worker-output");
    fs::create_dir_all(&output_root)?;
    let pdfium = Some(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local-archive/dependencies/pdfium-7881/bin/pdfium.dll")
            .canonicalize()?,
    );

    let mut db = Library::open(workspace)?;
    let workspace_id = db.info()?.workspace_id;
    let initial_metadata = db.workspace_metadata()?;
    db.update_workspace_metadata(pkms_core::UpdateWorkspaceMetadata {
        base_row_version: initial_metadata.row_version,
        object_type_registry: object_types_registry(&workspace_id)?,
        note_template_registry: note_templates_registry(&workspace_id)?,
    })?;

    let embedded = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "嵌入式开发".into(),
                overview_note_id: None,
                is_pinned: true,
                pin_position: Some(2),
            },
        )?
        .collection;
    let can = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "CAN 总线故障复盘".into(),
                overview_note_id: None,
                is_pinned: true,
                pin_position: Some(1),
            },
        )?
        .collection;
    let spi = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "SPI 时序与采样".into(),
                overview_note_id: None,
                is_pinned: false,
                pin_position: None,
            },
        )?
        .collection;
    let rtos = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "RTOS 调度与中断".into(),
                overview_note_id: None,
                is_pinned: false,
                pin_position: None,
            },
        )?
        .collection;
    let methods = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "研究方法与证据链".into(),
                overview_note_id: None,
                is_pinned: false,
                pin_position: None,
            },
        )?
        .collection;
    let people = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "人物与组织".into(),
                overview_note_id: None,
                is_pinned: false,
                pin_position: None,
            },
        )?
        .collection;
    let reading = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "阅读与摘录".into(),
                overview_note_id: None,
                is_pinned: true,
                pin_position: Some(3),
            },
        )?
        .collection;
    let project = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "项目复盘".into(),
                overview_note_id: None,
                is_pinned: true,
                pin_position: Some(0),
            },
        )?
        .collection;

    let rtos_critical_block = id();
    let rtos_note = create_note(
        &mut db,
        Some(&rtos.id),
        "RTOS 优先级反转：临界区长度与任务切换点",
        vec![
            frontmatter(
                "notes",
                "2026-10-20",
                &[
                    (
                        "summary",
                        "\"CAN RX 高优先级中断与 SPI DMA 完成回调的时序竞争\"",
                    ),
                    ("priority_before", "CAN_RX_1_SPI_DMA_5"),
                    ("priority_after", "CAN_RX_4_SPI_DMA_2"),
                ],
                &[],
            ),
            heading(2, "现场观察"),
            paragraph(
                "采样任务每 1 ms 触发一次，SPI DMA 完成回调负责切换缓冲区。CAN RX 突发时，回调最长被推迟 380 μs，超过从机数据稳定窗口。",
            ),
            paragraph_with_id(
                &rtos_critical_block,
                "最初把 SPI 丢帧归因于 CPOL/CPHA。示波器同步抓取显示 SPI 模式配置正确，真正的偏差发生在 DMA 完成中断进入时刻。",
            ),
            heading(2, "优先级配置"),
            bullet_list(&[
                "CAN RX 抢占优先级原为 1，高于 SPI DMA 完成中断的 5。",
                "SysTick 与采样任务使用 5，CAN 接收突发可以连续抢占。",
                "将 CAN RX 改为 4、SPI DMA 改为 2 后，延迟降到 41–63 μs。",
                "队列投递仍保留在 ISR 内，但不再执行日志格式化。",
            ]),
            callout(
                "warning",
                "判定规则",
                "临界区超过 50 μs 或 DMA 完成延迟超过 100 μs，必须视为潜在通信故障，而不是“偶发抖动”。",
            ),
            code_block(
                "c",
                "NVIC_SetPriority(SPI1_DMA_IRQn, 2);\nNVIC_SetPriority(CAN1_RX0_IRQn, 4);\nNVIC_SetPriority(SysTick_IRQn, 5);\n/* ISR 内只置位与投递，不格式化日志 */",
            ),
        ],
    )?;
    add_tags(
        &mut db,
        &rtos_note.id,
        &["RTOS", "优先级反转", "中断延迟", "CAN", "SPI"],
    )?;
    add_members(&mut db, &embedded.id, &[&rtos_note.id])?;
    set_inbox(&mut db, &rtos_note.id, InboxState::Processed)?;

    let person_quote_block = id();
    let zhou_note = create_note(
        &mut db,
        Some(&people.id),
        "周岚访谈：测量之前先固定时间基准",
        vec![
            frontmatter(
                "person",
                "2026-10-19",
                &[
                    ("organization", "测量与硬件验证组"),
                    ("role", "信号完整性与测量负责人"),
                    ("summary", "\"负责 CAN/SPI 联合抓包与探头校准\""),
                ],
                &[&rtos_note],
            ),
            heading(2, "访谈摘要"),
            paragraph(
                "周岚在复盘中指出，前期两份抓包看似显示采样点漂移，实际是触发源不同导致的时间轴错位。固定 SPI_CS 边沿作为触发后，DMA 延迟与 CAN RX 突发的关系才变得可量化。",
            ),
            paragraph_with_id(
                &person_quote_block,
                "她的核心要求是：所有证据先写明触发源、探头衰减、时间范围与固件哈希，再讨论波形是否异常。",
            ),
            blockquote(
                "“先固定时间基准，再检查物理层。否则你看到的是示波器的变化，不是控制板的变化。”",
            ),
            heading(2, "角色与关系"),
            bullet_list(&[
                "负责终端电阻、共模电压、SPI 时钟与 DMA 延迟的实测。",
                "参与 10 月 21 日评审，保留低温测试风险意见。",
                "后续行动：11 月 8 日前完成 −10℃ 启动测试。",
            ]),
        ],
    )?;
    add_tags(
        &mut db,
        &zhou_note.id,
        &["人物", "测量", "证据链", "SPI", "CAN"],
    )?;
    add_members(&mut db, &spi.id, &[&zhou_note.id])?;
    add_members(&mut db, &methods.id, &[&zhou_note.id])?;
    set_inbox(&mut db, &zhou_note.id, InboxState::Processed)?;

    let lin_note = create_note(
        &mut db,
        Some(&people.id),
        "林舟访谈：不要先怪总线",
        vec![
            frontmatter(
                "person",
                "2026-10-18",
                &[
                    ("organization", "嵌入式平台组"),
                    ("role", "控制板通信模块负责人"),
                    ("summary", "\"主导 CAN/SPI 故障复盘与中断优先级修复\""),
                ],
                &[&rtos_note],
            ),
            heading(2, "访谈摘要"),
            paragraph(
                "林舟负责控制板通信模块，主持了 10 月 21 日的故障复盘。他明确区分“总线物理层异常”和“主机中断时序异常”，并强调不能用更换线束掩盖可复现的软件时序问题。",
            ),
            reference_paragraph(
                "他在访谈中再次引用了当时的判断依据：",
                &rtos_note,
                Some(&rtos_critical_block),
                "。这段记录后来成为根因确认的关键证据。",
            ),
            blockquote("“不要先怪总线。先确认中断有没有把采样任务挤出去，再看物理层。”"),
            heading(2, "决策影响"),
            bullet_list(&[
                "把 CAN RX 与 SPI DMA 的优先级关系纳入 release 检查表。",
                "要求每次复现保留同一时间基准下的六路信号。",
                "拒绝仅通过降低 SPI 时钟来掩盖丢帧。",
            ]),
        ],
    )?;
    add_tags(
        &mut db,
        &lin_note.id,
        &["人物", "访谈", "CAN", "RTOS", "决策"],
    )?;
    add_members(&mut db, &project.id, &[&lin_note.id])?;
    add_members(&mut db, &rtos.id, &[&lin_note.id])?;
    set_inbox(&mut db, &lin_note.id, InboxState::Processed)?;

    let measure_note = create_note(
        &mut db,
        Some(&spi.id),
        "板级信号测量清单与接地检查",
        vec![
            frontmatter(
                "experiment-log",
                "2026-10-17",
                &[
                    ("summary", "\"SPI 与 CAN 同步测量的探头、触发和接地清单\""),
                    ("test_window", "\"2026-10-17 14:00–17:30\""),
                    ("result", "pass"),
                    ("measured_at", "2026-10-17"),
                ],
                &[],
            ),
            heading(2, "测量条件"),
            bullet_list(&[
                "SPI 时钟 16 MHz，模式 1，片选建立时间 160 ns。",
                "CAN 500 kbit/s，采样点先按 87.5% 记录，后调整为 80%。",
                "逻辑分析仪采样率 200 MS/s，探头地线长度小于 8 cm。",
                "所有测量以 SPI_CS 上升沿为统一触发。",
            ]),
            heading(2, "核对结果"),
            bullet_list(&[
                "CAN_H 峰值 3.38 V，CAN_L 谷值 1.10 V，差分信号在规格内。",
                "终端等效电阻 60.1 Ω，两端各 120 Ω，无第三终端。",
                "SPI 采样边沿位于数据稳定区，问题不在时钟极性。",
                "DMA 完成中断延迟与 CAN RX 突发呈同步关系。",
            ]),
            callout(
                "info",
                "保留证据",
                "设备序列号、探头衰减、触发源和固件哈希必须与抓包文件一起归档；缺失任何一项都不可作为根因证据。",
            ),
        ],
    )?;
    add_tags(
        &mut db,
        &measure_note.id,
        &["测量", "SPI", "CAN", "证据链", "实验"],
    )?;
    add_members(&mut db, &methods.id, &[&measure_note.id])?;
    set_inbox(&mut db, &measure_note.id, InboxState::Processed)?;

    let spi_note = create_note(
        &mut db,
        Some(&spi.id),
        "SPI DMA 完成中断与 CPOL/CPHA 时序核对",
        vec![
            frontmatter(
                "experiment-log",
                "2026-10-20",
                &[
                    ("summary", "\"排除 SPI 模式错误，确认 DMA 完成回调延迟\""),
                    ("test_window", "\"2026-10-20 09:30–12:10\""),
                    ("result", "fail"),
                    ("measured_at", "2026-10-20"),
                ],
                &[&rtos_note, &measure_note],
            ),
            heading(2, "假设与对照"),
            paragraph(
                "初始假设是 CPOL/CPHA 与从机数据手册不一致。核对配置寄存器和六路波形后，确认主机与从机均工作在模式 1，采样边沿位于数据稳定区。",
            ),
            bullet_list(&[
                "SPI 模式 1：空闲低电平，第二个时钟边沿采样。",
                "片选建立时间 160 ns，释放后保留 1 字节时钟。",
                "从机返回 0x00 时，DMA 完成中断已延迟 300 μs 以上。",
                "将 CAN RX 中断临时屏蔽后，丢帧消失。",
            ]),
            heading(2, "根因指向"),
            paragraph(
                "SPI 配置不是根因。DMA 完成中断被 CAN RX 连续抢占，双缓冲切换晚于下一个采样窗口，导致读取旧数据或被覆盖的数据。",
            ),
            reference_paragraph("测量条件与判定线见：", &measure_note, None, "。"),
        ],
    )?;
    add_tags(
        &mut db,
        &spi_note.id,
        &["SPI", "DMA", "中断延迟", "采样点", "实验"],
    )?;
    add_members(&mut db, &embedded.id, &[&spi_note.id])?;
    add_members(&mut db, &rtos.id, &[&spi_note.id])?;
    set_inbox(&mut db, &spi_note.id, InboxState::Processed)?;

    let observation_note = create_note(
        &mut db,
        Some(&can.id),
        "六小时复测日志索引：7 个观察点",
        vec![
            frontmatter(
                "experiment-log",
                "2026-10-22",
                &[
                    ("summary", "\"六小时闭环保测的观察点、阈值与异常说明\""),
                    ("test_window", "\"2026-10-22 00:00–06:00\""),
                    ("result", "pass"),
                    ("measured_at", "2026-10-22"),
                ],
                &[&spi_note],
            ),
            heading(2, "观察点"),
            task_list(&[
                ("15 分钟：错误帧 0，SPI 丢帧 0", true),
                ("60 分钟：负载 58.4%，最大 DMA 延迟 52 μs", true),
                ("120 分钟：从机看门狗复位一次，独立立案", true),
                ("180 分钟：负载 61.8%，错误帧仍为 0", true),
                ("300 分钟：最大延迟 60 μs", true),
                ("360 分钟：错误帧 0，SPI 丢帧 0，验收通过", true),
            ]),
            paragraph(
                "02:00 的从机看门狗复位由测试电源瞬态触发，数据流在 2.1 s 后恢复。该事件没有被忽略，但没有产生 CAN 错误帧，因此不计入通信验收统计。",
            ),
            callout(
                "success",
                "闭环条件",
                "连续 6 小时错误帧 0、SPI 丢帧 0，且最大 DMA 完成延迟小于 100 μs；三项缺一不可。",
            ),
        ],
    )?;
    add_tags(
        &mut db,
        &observation_note.id,
        &["六小时", "CAN", "SPI", "证据", "实验"],
    )?;
    add_members(&mut db, &project.id, &[&observation_note.id])?;
    set_inbox(&mut db, &observation_note.id, InboxState::Processed)?;

    let can_note = create_note(
        &mut db,
        Some(&can.id),
        "CAN 终端电阻与采样点联合排查",
        vec![
            frontmatter(
                "fault-case",
                "2026-10-18",
                &[
                    (
                        "summary",
                        "\"排除终端电阻假设，定位到采样点与中断时序的耦合\"",
                    ),
                    ("severity", "high"),
                    ("review_phase", "root-cause"),
                    ("occurred_on", "2026-10-14"),
                ],
                &[&spi_note, &observation_note],
            ),
            heading(2, "现场现象"),
            paragraph(
                "控制板在 500 kbit/s 波特率下偶发错误帧。总线负载超过 62% 后，主站每小时平均记录 14 个错误帧，最高 17 个；同时 SPI 从机偶发返回 0x00。",
            ),
            heading(2, "假设检验"),
            bullet_list(&[
                "终端电阻：两端各 120 Ω，并联 60.1 Ω，符合预期。",
                "第三终端：移除备用终端后错误帧未下降，假设被证伪。",
                "采样点：87.5% 对当前线束裕量不足，调整为 80%。",
                "中断时序：CAN RX 抢占 SPI DMA，是错误帧与丢帧同步出现的关键。",
            ]),
            heading(2, "结论"),
            paragraph(
                "终端电阻与采样点是必要检查项，但本次主因是中断时序。采样点调整提高了物理层裕量，真正让故障归零的是中断优先级与 DMA 双缓冲修复。",
            ),
            reference_paragraph("完整六小时观察点见：", &observation_note, None, "。"),
        ],
    )?;
    add_tags(
        &mut db,
        &can_note.id,
        &["CAN", "终端电阻", "采样点", "故障排查", "六小时"],
    )?;
    add_members(&mut db, &embedded.id, &[&can_note.id])?;
    add_members(&mut db, &project.id, &[&can_note.id])?;
    set_inbox(&mut db, &can_note.id, InboxState::Processed)?;

    let ddia_note = create_note(
        &mut db,
        Some(&reading.id),
        "设计数据密集型应用第 128 页阅读笔记",
        vec![
            frontmatter(
                "book",
                "2026-10-12",
                &[
                    (
                        "summary",
                        "\"复制延迟不是异常，而是系统必须暴露给使用者的状态\"",
                    ),
                    ("reading_status", "finished"),
                    ("page", "128"),
                ],
                &[],
            ),
            heading(2, "原文摘录"),
            blockquote(
                "复制延迟不是异常，而是系统设计必须暴露给使用者的状态。一个知识库如果声称“已经同步”，却不显示读写快照、索引版本和失败回退，就会把系统设计问题伪装成用户操作问题。",
            ),
            heading(2, "迁移到知识库"),
            bullet_list(&[
                "搜索结果应显示命中的资料版本，而不是只显示当前标题。",
                "摘录必须保留页码或文本块定位；来源被替换后要明确标为近似定位。",
                "语义索引未配置时，关键词检索仍可用，并明确说明降级原因。",
                "多设备同步不能用一个模糊的“同步中”概括离线、冲突和待确认。",
            ]),
        ],
    )?;
    add_tags(&mut db, &ddia_note.id, &["阅读", "摘录", "一致性", "同步"])?;
    set_inbox(&mut db, &ddia_note.id, InboxState::Processed)?;

    let scheduling_note = create_note(
        &mut db,
        Some(&reading.id),
        "实时系统调度讲义第 4 章：抢占与优先级继承",
        vec![
            frontmatter(
                "book",
                "2026-10-16",
                &[
                    (
                        "summary",
                        "\"优先级只定义抢占顺序，不能替代对临界区与共享资源的分析\"",
                    ),
                    ("reading_status", "finished"),
                    ("page", "74–91"),
                ],
                &[&rtos_note],
            ),
            heading(2, "关键概念"),
            paragraph(
                "固定优先级抢占式调度可以降低高优先级任务的响应延迟，但如果低优先级任务持有共享资源，仍会出现无界阻塞。优先级继承只能缩短阻塞，不能消除过长临界区。",
            ),
            bullet_list(&[
                "最坏响应时间必须包含中断抢占与临界区等待。",
                "优先级继承只在共享资源处生效，不改变 ISR 抢占关系。",
                "测量必须同时记录调度抖动与中断进入延迟。",
            ]),
            reference_paragraph("与本次故障相关的实测记录：", &rtos_note, None, "。"),
        ],
    )?;
    add_tags(
        &mut db,
        &scheduling_note.id,
        &["阅读", "RTOS", "优先级继承", "调度"],
    )?;
    add_members(&mut db, &rtos.id, &[&scheduling_note.id])?;
    set_inbox(&mut db, &scheduling_note.id, InboxState::Processed)?;

    let later_note = create_note(
        &mut db,
        Some(&reading.id),
        "稍后阅读：CAN FD 误码率与接地设计资料清单",
        vec![
            frontmatter(
                "book",
                "2026-10-23",
                &[
                    ("summary", "\"CAN FD 高速段误码率与地偏移的补充阅读清单\""),
                    ("reading_status", "want"),
                ],
                &[&can_note],
            ),
            heading(2, "待读主题"),
            bullet_list(&[
                "CAN FD 数据段 5 Mbit/s 下的振铃与终端网络。",
                "多点接地、单点接地与共模扼流圈对误码率的影响。",
                "示波器探头地线长度对高速边沿测量的偏差。",
            ]),
            paragraph(
                "这份清单暂不进入本次故障结论。它是下一轮低温与长线束测试前的资料准备，避免把未读材料误当成已验证依据。",
            ),
        ],
    )?;
    add_tags(&mut db, &later_note.id, &["稍后", "CAN", "阅读", "接地"])?;
    add_members(&mut db, &can.id, &[&later_note.id])?;
    set_inbox(&mut db, &later_note.id, InboxState::Snoozed)?;

    let evidence_block = id();
    let n1_initial_children = vec![
        frontmatter(
            "fault-case",
            "2026-10-21",
            &[
                (
                    "summary",
                    "\"从每小时 14 个错误帧到连续 6 小时零错误的完整复盘\"",
                ),
                ("severity", "high"),
                ("review_phase", "verified"),
                ("occurred_on", "2026-10-14"),
                ("duration_hours", "6"),
            ],
            &[&can_note, &spi_note, &rtos_note, &lin_note],
        ),
        heading(2, "执行摘要"),
        paragraph(
            "北岭-4 控制板在电机启动后的 300 ms 窗口内出现 SPI 从机返回 0x00，同时 CAN 错误帧上升。修复前平均每小时 14 个错误帧；修复后连续 6 小时为 0。",
        ),
        paragraph_with_id(
            &evidence_block,
            "根因不是终端电阻，而是 CAN RX 中断抢占 SPI DMA 完成中断。终端电阻并联值 60.1 Ω，SPI 模式 1 配置正确。",
        ),
        heading(2, "修复动作"),
        bullet_list(&[
            "CAN RX 抢占优先级从 1 调整为 4，SPI DMA 完成中断提升到 2。",
            "最长临界区从 91 μs 缩短到 17 μs。",
            "DMA 改为双缓冲，半缓冲事件只做指针切换。",
            "CAN 采样点调整为 80%，网关改为事件驱动。",
        ]),
        heading(2, "验证结果"),
        paragraph(
            "六小时闭环保测：错误帧 0，SPI 丢帧 0，最大 DMA 完成延迟 63 μs，调度抖动 0.82 ms，总线负载峰值 61.8%。",
        ),
        reference_paragraph("访谈中的原始判断：", &lin_note, None, "。"),
        heading(2, "遗留风险"),
        bullet_list(&[
            "−10℃ 低温启动尚未覆盖，计划 11 月 8 日前补测。",
            "网关队列峰值仍可达到 18 帧，需要增加背压指标。",
            "从机看门狗在 02:00 复位一次，已独立立案跟踪。",
        ]),
    ];
    let mut main_note = create_note(
        &mut db,
        Some(&project.id),
        "控制板通信故障复盘：从每小时 14 个错误帧到连续 6 小时零错误",
        n1_initial_children.clone(),
    )?;
    add_tags(
        &mut db,
        &main_note.id,
        &["CAN", "SPI", "RTOS", "复盘", "决策", "六小时"],
    )?;
    add_members(&mut db, &embedded.id, &[&main_note.id])?;
    add_members(&mut db, &can.id, &[&main_note.id])?;
    add_members(&mut db, &spi.id, &[&main_note.id])?;
    add_members(&mut db, &rtos.id, &[&main_note.id])?;
    set_inbox(&mut db, &main_note.id, InboxState::Processed)?;

    let evidence_note = create_note(
        &mut db,
        Some(&methods.id),
        "故障证据链与复现记录规范",
        vec![
            frontmatter(
                "notes",
                "2026-10-21",
                &[
                    (
                        "summary",
                        "\"故障结论必须可回到原始证据、时间基准和修复版本\"",
                    ),
                    ("version", "1.1"),
                ],
                &[&main_note, &zhou_note],
            ),
            heading(2, "最小证据集"),
            bullet_list(&[
                "原始抓包与逻辑分析仪文件，不使用截图代替原始数据。",
                "固定触发源、设备序列号、固件哈希与测试环境。",
                "故障前、修复后使用同一测量口径与同一负载曲线。",
                "错误计数快照必须包含 TEC/REC 与 Error Passive 进入时刻。",
            ]),
            reference_paragraph(
                "主复盘中的关键判断：",
                &main_note,
                Some(&evidence_block),
                "。",
            ),
            paragraph(
                "这条规范要求结论能回到具体观测点。只保留“已修复”的摘要，不保留原始证据，无法在两周后判断修复是否仍然有效。",
            ),
            callout(
                "info",
                "写作规则",
                "先写事实与观测值，再写解释；把反例、未覆盖条件和失败路径保留在同一篇复盘里。",
            ),
        ],
    )?;
    add_tags(
        &mut db,
        &evidence_note.id,
        &["证据链", "复现", "方法", "复盘"],
    )?;
    add_members(&mut db, &project.id, &[&evidence_note.id])?;
    add_members(&mut db, &people.id, &[&evidence_note.id])?;
    set_inbox(&mut db, &evidence_note.id, InboxState::Processed)?;

    let mut revised_children = n1_initial_children;
    revised_children.extend([
        heading(2, "证据链与后续阅读"),
        reference_paragraph(
            "证据规范见：",
            &evidence_note,
            None,
            "。该笔记解释了为什么必须保留原始抓包、固定时间基准，而不是只写结论。",
        ),
    ]);
    main_note = db
        .save_document(
            &id(),
            SaveNote {
                note_id: main_note.id.clone(),
                base_revision_id: main_note.revision_id.clone(),
                title: main_note.title.clone(),
                document_schema: 3,
                document: document(revised_children),
            },
        )?
        .note;
    add_members(&mut db, &methods.id, &[&main_note.id])?;

    let risk_note = create_note(
        &mut db,
        Some(&project.id),
        "项目风险清单与修复决策",
        vec![
            frontmatter(
                "fault-case",
                "2026-10-21",
                &[
                    ("summary", "\"记录已接受修复、保留风险与重开条件\""),
                    ("severity", "medium"),
                    ("review_phase", "verified"),
                    ("occurred_on", "2026-10-21"),
                ],
                &[&main_note, &evidence_note],
            ),
            heading(2, "已接受决策"),
            bullet_list(&[
                "D-01：采用当前中断优先级与 DMA 双缓冲方案。",
                "D-02：六小时内错误帧 0、SPI 丢帧 0、最大延迟小于 100 μs，方可关闭现场问题。",
                "D-03：低温、长期老化和从机复位继续观察，不把未覆盖项写成通过。",
            ]),
            heading(2, "风险登记"),
            bullet_list(&[
                "R-01：低温 −10℃ 启动未测试，负责人周岚，截止 2026-11-08。",
                "R-02：网关队列峰值 18 帧，负责人唐弈，截止 2026-11-12。",
                "R-03：四周观察期内出现 1 个错误帧即重开复盘。",
            ]),
            reference_paragraph("重开条件以主复盘为准：", &main_note, None, "。"),
            callout(
                "warning",
                "门禁",
                "修复版本不能在风险清单未签字前进入量产；观察期内任何错误帧都视为回归。",
            ),
        ],
    )?;
    add_tags(
        &mut db,
        &risk_note.id,
        &["风险", "决策", "复盘", "量产门禁"],
    )?;
    add_members(&mut db, &can.id, &[&risk_note.id])?;
    add_members(&mut db, &methods.id, &[&risk_note.id])?;
    set_inbox(&mut db, &risk_note.id, InboxState::Processed)?;

    let meeting_note = create_note(
        &mut db,
        Some(&project.id),
        "会议纪要：10 月 21 日故障复盘",
        vec![
            frontmatter(
                "review-meeting",
                "2026-10-21",
                &[
                    ("summary", "\"确认根因、修复门禁、行动项与保留意见\""),
                    ("meeting_date", "2026-10-21"),
                    ("participants", "\"林舟、周岚、唐弈、沈禾\""),
                    ("decision_status", "accepted"),
                ],
                &[&main_note, &lin_note],
            ),
            heading(2, "证据复核"),
            paragraph(
                "会议复核六路同步抓包、DMA 延迟计数、终端电阻测量和 2 小时预验证。确认主因是中断时序，终端电阻正常。",
            ),
            reference_paragraph("主复盘材料：", &main_note, None, "；"),
            reference_paragraph("访谈中的原始判断：", &lin_note, None, "。"),
            heading(2, "行动项"),
            task_list(&[
                ("A-01：完成六小时闭环保测并归档抓包，林舟，10-22", true),
                ("A-02：补充 −10℃ 低温启动测试，周岚，11-08", false),
                ("A-03：网关增加队列背压指标与告警，唐弈，11-12", false),
                ("A-04：更新联合排查手册，林舟，10-30", false),
            ]),
            callout(
                "warning",
                "保留意见",
                "沈禾提出：六小时测试不能覆盖低温与老化；02:00 从机复位必须独立跟踪，不能从复盘中删除。",
            ),
        ],
    )?;
    add_tags(
        &mut db,
        &meeting_note.id,
        &["会议纪要", "决策", "行动项", "证据链"],
    )?;
    add_members(&mut db, &people.id, &[&meeting_note.id])?;
    add_members(&mut db, &methods.id, &[&meeting_note.id])?;
    set_inbox(&mut db, &meeting_note.id, InboxState::Processed)?;

    let inbox_note = create_note(
        &mut db,
        None,
        "待整理：控制板通信稳定性评审遗留项",
        vec![
            frontmatter(
                "notes",
                "2026-10-26",
                &[
                    ("summary", "\"尚未归类的三条现场遗留项，等待下一次评审\""),
                    ("box", "inbox"),
                ],
                &[&meeting_note, &observation_note],
            ),
            heading(2, "遗留项"),
            bullet_list(&[
                "恢复 02:00 看门狗复位前后的完整供电波形，确认是否还有其他瞬态。",
                "核对网关队列峰值 18 帧时的报文优先级，判断是否需要分区发送。",
                "把六小时观察点加入 release 验收模板，避免只记录起点和终点。",
            ]),
            paragraph("本笔记暂不归属分类，用于测试收件箱、批量整理和稍后处理。"),
        ],
    )?;
    add_tags(&mut db, &inbox_note.id, &["待整理", "电源瞬态", "网关"])?;
    set_inbox(&mut db, &inbox_note.id, InboxState::Pending)?;

    let guide_note = create_note(
        &mut db,
        Some(&methods.id),
        "演示库说明与验收路线",
        vec![
            frontmatter(
                "notes",
                "2026-10-29",
                &[
                    (
                        "summary",
                        "\"本库是真实技术结构的演示数据，按页面给出可验证路径\"",
                    ),
                    ("box", "reference"),
                ],
                &[&main_note, &evidence_note],
            ),
            heading(2, "数据说明"),
            paragraph(
                "本库围绕北岭-4 控制板通信稳定性专项组织，人员与项目为虚构，但技术路径、指标、证据链和文档内容自洽。它不对应真实客户或真实组织。",
            ),
            heading(2, "验收入口"),
            bullet_list(&[
                "精确检索：搜索“不要先怪总线”或“每小时 14 个错误帧”。",
                "模糊检索：搜索“为什么采样点会漂移”或“谁负责低温测试”。",
                "收件箱：查看“待整理：控制板通信稳定性评审遗留项”。",
                "资料：打开 PDF 第 5–6 页、Markdown 手册和 CSV 六小时统计。",
                "画布：打开“控制板通信故障关系图”，检查节点尺寸与关系边。",
                "视图：查看看板、日历、时间线、表格四个已保存视图。",
                "回顾：到期项包含修复验证和访谈补充；稍后项是来源复查。",
                "自动化：保留一组待审阅建议与一次可撤销审计事件。",
                "回收站：包含一个错误假设笔记和一份废弃草稿。",
            ]),
            callout(
                "info",
                "边界",
                "模型、同步和远程 AI 凭据均未配置，不应伪造成功状态；相关页面应显示未配置或不可用说明。",
            ),
        ],
    )?;
    add_tags(&mut db, &guide_note.id, &["验收", "检索", "方法", "演示库"])?;
    set_inbox(&mut db, &guide_note.id, InboxState::Processed)?;

    let trashed_note = create_note(
        &mut db,
        Some(&can.id),
        "旧版假设：全部是终端电阻问题（已证伪）",
        vec![
            frontmatter(
                "fault-case",
                "2026-10-15",
                &[
                    ("summary", "\"保留被证伪的终端电阻假设，避免重复走错路\""),
                    ("severity", "low"),
                    ("review_phase", "observed"),
                    ("occurred_on", "2026-10-15"),
                ],
                &[&can_note],
            ),
            heading(2, "原假设"),
            paragraph("错误帧可能由第三个终端电阻造成。计划更换线束并移除备用终端。"),
            heading(2, "证伪过程"),
            bullet_list(&[
                "实测并联电阻 60.1 Ω，符合两端各 120 Ω 的预期。",
                "移除备用终端后错误帧没有下降。",
                "固定触发源后确认延迟发生在 CAN RX 抢占期间。",
            ]),
            callout(
                "warning",
                "归档原因",
                "保留错误路径用于训练排查判断，避免把“更换硬件后偶发消失”误当成根因。",
            ),
        ],
    )?;
    add_tags(
        &mut db,
        &trashed_note.id,
        &["CAN", "终端电阻", "已证伪", "错误路径"],
    )?;
    trash_object(&mut db, &trashed_note.id)?;

    let overview_updates = [
        (&embedded.id, &rtos_note.id, "嵌入式开发"),
        (&can.id, &can_note.id, "CAN 总线故障复盘"),
        (&spi.id, &spi_note.id, "SPI 时序与采样"),
        (&rtos.id, &rtos_note.id, "RTOS 调度与中断"),
        (&methods.id, &evidence_note.id, "研究方法与证据链"),
        (&people.id, &lin_note.id, "人物与组织"),
        (&reading.id, &ddia_note.id, "阅读与摘录"),
        (&project.id, &main_note.id, "项目复盘"),
    ];
    for (collection_id, overview_id, name) in overview_updates {
        let current = collection(&db, collection_id)?;
        db.collection_update(
            &id(),
            pkms_core::UpdateCollection {
                collection_id: collection_id.clone(),
                base_row_version: current.metadata.row_version,
                name: name.to_owned(),
                overview_note_id: Some(overview_id.clone()),
                is_pinned: current.is_pinned,
                pin_position: current.pin_position,
            },
        )?;
    }

    let (pdf_source, pdf_version, pdf_parsed) = import_file(
        &mut db,
        &fixtures.join("控制板通信稳定性复盘-2026-10-21.pdf"),
        &output_root,
        SourceFormat::Pdf,
        Some(&project.id),
        None,
        pdfium.as_deref(),
    )?;
    let (manual_source, manual_version, _) = import_file(
        &mut db,
        &fixtures.join("SPI与CAN联合排查手册.md"),
        &output_root,
        SourceFormat::Markdown,
        Some(&spi.id),
        None,
        pdfium.as_deref(),
    )?;
    let (timeline_source, timeline_version, _) = import_file(
        &mut db,
        &fixtures.join("CAN抓包时间线-六小时复测.txt"),
        &output_root,
        SourceFormat::Text,
        Some(&can.id),
        None,
        pdfium.as_deref(),
    )?;
    let (dashboard_source, dashboard_version, _) = import_file(
        &mut db,
        &fixtures.join("控制板通信故障看板.html"),
        &output_root,
        SourceFormat::Html,
        Some(&project.id),
        None,
        pdfium.as_deref(),
    )?;

    let (minutes_source, minutes_version, _) = import_file(
        &mut db,
        &fixtures.join("评审纪要-控制板通信稳定性.docx"),
        &output_root,
        SourceFormat::Docx,
        Some(&project.id),
        None,
        pdfium.as_deref(),
    )?;
    let (csv_source, csv_version, _) = import_file(
        &mut db,
        &fixtures.join("六小时总线错误帧统计.csv"),
        &output_root,
        SourceFormat::Csv,
        Some(&can.id),
        None,
        pdfium.as_deref(),
    )?;
    let (xlsx_source, xlsx_version, _) = import_file(
        &mut db,
        &fixtures.join("故障记录与信号测量.xlsx"),
        &output_root,
        SourceFormat::Xlsx,
        Some(&can.id),
        None,
        pdfium.as_deref(),
    )?;
    let (pptx_source, pptx_version, _) = import_file(
        &mut db,
        &fixtures.join("控制板通信故障复盘汇报.pptx"),
        &output_root,
        SourceFormat::Pptx,
        Some(&project.id),
        None,
        pdfium.as_deref(),
    )?;
    let (supplement_source, _supplement_version, _) = import_file(
        &mut db,
        &fixtures.join("现场补充-从机电源瞬态记录.txt"),
        &output_root,
        SourceFormat::Text,
        None,
        None,
        pdfium.as_deref(),
    )?;
    let (waste_source, _waste_version, _) = import_file(
        &mut db,
        &fixtures.join("废弃-CAN终端电阻草稿.txt"),
        &output_root,
        SourceFormat::Text,
        Some(&can.id),
        None,
        pdfium.as_deref(),
    )?;

    let source_tags: [(&str, &[&str]); 10] = [
        (&pdf_source.id, &["CAN", "证据链", "复盘报告", "摘录"]),
        (&manual_source.id, &["SPI", "CAN", "排查手册", "方法"]),
        (&timeline_source.id, &["CAN", "时间线", "六小时", "证据"]),
        (&dashboard_source.id, &["看板", "CAN", "统计", "可视化"]),
        (&minutes_source.id, &["会议纪要", "决策", "评审", "CAN"]),
        (&csv_source.id, &["CSV", "统计", "六小时", "证据"]),
        (&xlsx_source.id, &["XLSX", "测量", "故障记录", "CAN"]),
        (&pptx_source.id, &["汇报", "复盘", "CAN", "决策"]),
        (&supplement_source.id, &["待整理", "电源瞬态"]),
        (&waste_source.id, &["CAN", "终端电阻", "已证伪"]),
    ];
    for (source_id, tags) in source_tags {
        add_tags(&mut db, source_id, tags)?;
    }

    add_members(
        &mut db,
        &can.id,
        &[
            &pdf_source.id,
            &timeline_source.id,
            &xlsx_source.id,
            &csv_source.id,
        ],
    )?;
    add_members(&mut db, &spi.id, &[&manual_source.id, &pdf_source.id])?;
    add_members(
        &mut db,
        &methods.id,
        &[&manual_source.id, &pdf_source.id, &minutes_source.id],
    )?;
    add_members(&mut db, &people.id, &[&minutes_source.id])?;
    add_members(
        &mut db,
        &project.id,
        &[&timeline_source.id, &csv_source.id, &xlsx_source.id],
    )?;

    for source in [
        &pdf_source,
        &manual_source,
        &timeline_source,
        &dashboard_source,
        &minutes_source,
        &csv_source,
        &xlsx_source,
        &pptx_source,
        &waste_source,
    ] {
        set_inbox(&mut db, &source.id, InboxState::Processed)?;
    }
    set_inbox(&mut db, &supplement_source.id, InboxState::Pending)?;

    let (pdf_annotation, pdf_page) = create_annotation_for(
        &mut db,
        &pdf_version,
        true,
        &["根因不是终端电阻", "根因", "CAN"],
        "报告的因果链从现象、证据到修复结果完整，可作为复盘模板。",
    )?;
    let (manual_annotation, _) = create_annotation_for(
        &mut db,
        &manual_version,
        false,
        &["先固定时间基准，再检查物理层。", "先固定时间基准"],
        "这条规则应在每次联合排查前复述。",
    )?;
    let (timeline_annotation, _) = create_annotation_for(
        &mut db,
        &timeline_version,
        false,
        &["第 6 小时错误计数保持 0", "错误计数保持 0"],
        "六小时闭环的终点证据。",
    )?;
    let (dashboard_annotation, _) = create_annotation_for(
        &mut db,
        &dashboard_version,
        false,
        &["每小时错误帧数从 14 降到 0", "从 14 降到 0"],
        "看板摘要与统计表一致。",
    )?;

    let (minutes_annotation, _) = create_annotation_for(
        &mut db,
        &minutes_version,
        false,
        &["修复后必须连续覆盖 6 小时且错误帧为 0", "连续覆盖 6 小时"],
        "评审门禁，后续行动要引用。",
    )?;
    let (csv_annotation, _) = create_annotation_for(
        &mut db,
        &csv_version,
        false,
        &["合格"],
        "最后一个观察点判定为合格。",
    )?;
    let (xlsx_annotation, _) = create_annotation_for(
        &mut db,
        &xlsx_version,
        false,
        &["看门狗复位"],
        "从机复位与错误帧无关，但需独立跟踪。",
    )?;
    let (pptx_annotation, _) = create_annotation_for(
        &mut db,
        &pptx_version,
        false,
        &["先修中断优先级", "终端电阻"],
        "汇报页的核心结论与报告一致。",
    )?;

    let annotations: [(&pkms_core::Annotation, &str, &[&str]); 8] = [
        (&pdf_annotation, &project.id, &["摘录", "证据链", "PDF"]),
        (
            &manual_annotation,
            &methods.id,
            &["摘录", "方法", "时间基准"],
        ),
        (&timeline_annotation, &can.id, &["摘录", "六小时"]),
        (&dashboard_annotation, &project.id, &["摘录", "看板"]),
        (&minutes_annotation, &project.id, &["摘录", "决策"]),
        (&csv_annotation, &can.id, &["摘录", "统计"]),
        (&xlsx_annotation, &can.id, &["摘录", "测量"]),
        (&pptx_annotation, &project.id, &["摘录", "汇报"]),
    ];
    for (annotation, collection_id, tags) in annotations {
        add_tags(&mut db, &annotation.id, tags)?;
        add_members(&mut db, collection_id, &[&annotation.id])?;
        set_inbox(&mut db, &annotation.id, InboxState::Pending)?;
    }
    for annotation in [
        &pdf_annotation,
        &manual_annotation,
        &timeline_annotation,
        &csv_annotation,
    ] {
        set_inbox(&mut db, &annotation.id, InboxState::Processed)?;
    }

    let pdf_reading_page = pdf_page.unwrap_or(0);
    db.set_reading_state(
        &id(),
        SetReadingState {
            source_id: pdf_source.id.clone(),
            source_version_id: pdf_version.id.clone(),
            locator: Some(pdf_annotation.locator.clone()),
            status: ReadingStatus::Reading,
            base_row_version: 0,
            pdf_view: Some(PdfReadingView {
                page_index: pdf_reading_page,
                anchor_x: 0.2,
                anchor_y: 0.35,
                scale: 1.0,
                rotation: 0,
                zoom_mode: PdfZoomMode::Custom,
            }),
        },
    )?;
    db.set_reading_state(
        &id(),
        SetReadingState {
            source_id: manual_source.id.clone(),
            source_version_id: manual_version.id.clone(),
            locator: Some(manual_annotation.locator.clone()),
            status: ReadingStatus::Finished,
            base_row_version: 0,
            pdf_view: None,
        },
    )?;
    db.set_reading_state(
        &id(),
        SetReadingState {
            source_id: timeline_source.id.clone(),
            source_version_id: timeline_version.id.clone(),
            locator: Some(timeline_annotation.locator.clone()),
            status: ReadingStatus::Finished,
            base_row_version: 0,
            pdf_view: None,
        },
    )?;
    db.set_reading_state(
        &id(),
        SetReadingState {
            source_id: xlsx_source.id.clone(),
            source_version_id: xlsx_version.id.clone(),
            locator: None,
            status: ReadingStatus::Reading,
            base_row_version: 0,
            pdf_view: None,
        },
    )?;

    let source_note = db
        .create_source_note(
            &id(),
            CreateSourceNote {
                source_version_id: manual_version.id.clone(),
                source_blob_hash: manual_version.blob_hash.clone(),
                title: "从原始手册创建的编辑笔记：联合排查手册工作副本".into(),
                document_schema: 3,
                document: document(vec![
                    frontmatter(
                        "notes",
                        "2026-10-22",
                        &[
                            ("summary", "\"由不可变 Markdown 原件创建的编辑副本\""),
                            ("source", "SPI与CAN联合排查手册.md"),
                        ],
                        &[&main_note],
                    ),
                    heading(2, "工作副本用途"),
                    paragraph("这份笔记从 Markdown 原件创建，用于验证原件不可变、编辑副本与来源引用同时存在。正文仍可继续编辑，但不会覆盖原始资料。"),
                    paragraph("排查前先固定时间基准；所有结论必须能回到记录点。"),
                ]),
            },
        )?
        .note;
    add_tags(
        &mut db,
        &source_note.id,
        &["来源笔记", "SPI", "CAN", "方法"],
    )?;
    add_members(&mut db, &spi.id, &[&source_note.id])?;
    add_members(&mut db, &methods.id, &[&source_note.id])?;
    set_inbox(&mut db, &source_note.id, InboxState::Processed)?;

    let canvas_initial = db
        .canvas_save(
            &id(),
            SaveCanvas {
                canvas_id: None,
                base_row_version: None,
                name: "控制板通信故障关系图".into(),
                config_schema: 1,
                nodes: vec![
                    CanvasNode {
                        note_id: main_note.id.clone(),
                        x: 440.0,
                        y: 80.0,
                        width: 320.0,
                        height: 190.0,
                        z_index: 4,
                    },
                    CanvasNode {
                        note_id: can_note.id.clone(),
                        x: 80.0,
                        y: 80.0,
                        width: 280.0,
                        height: 170.0,
                        z_index: 2,
                    },
                    CanvasNode {
                        note_id: spi_note.id.clone(),
                        x: 80.0,
                        y: 330.0,
                        width: 280.0,
                        height: 170.0,
                        z_index: 1,
                    },
                    CanvasNode {
                        note_id: rtos_note.id.clone(),
                        x: 440.0,
                        y: 330.0,
                        width: 320.0,
                        height: 180.0,
                        z_index: 3,
                    },
                    CanvasNode {
                        note_id: lin_note.id.clone(),
                        x: 840.0,
                        y: 80.0,
                        width: 280.0,
                        height: 170.0,
                        z_index: 1,
                    },
                    CanvasNode {
                        note_id: evidence_note.id.clone(),
                        x: 840.0,
                        y: 330.0,
                        width: 280.0,
                        height: 170.0,
                        z_index: 2,
                    },
                    CanvasNode {
                        note_id: risk_note.id.clone(),
                        x: 440.0,
                        y: 590.0,
                        width: 320.0,
                        height: 180.0,
                        z_index: 5,
                    },
                ],
                is_pinned: true,
                pin_position: Some(0),
            },
        )?
        .canvas;
    let canvas = db
        .canvas_save(
            &id(),
            SaveCanvas {
                canvas_id: Some(canvas_initial.id.clone()),
                base_row_version: Some(canvas_initial.row_version),
                name: canvas_initial.name.clone(),
                config_schema: 1,
                nodes: vec![
                    CanvasNode {
                        note_id: main_note.id.clone(),
                        x: 450.0,
                        y: 70.0,
                        width: 340.0,
                        height: 205.0,
                        z_index: 4,
                    },
                    CanvasNode {
                        note_id: can_note.id.clone(),
                        x: 70.0,
                        y: 70.0,
                        width: 300.0,
                        height: 180.0,
                        z_index: 2,
                    },
                    CanvasNode {
                        note_id: spi_note.id.clone(),
                        x: 70.0,
                        y: 340.0,
                        width: 300.0,
                        height: 180.0,
                        z_index: 1,
                    },
                    CanvasNode {
                        note_id: rtos_note.id.clone(),
                        x: 450.0,
                        y: 340.0,
                        width: 340.0,
                        height: 190.0,
                        z_index: 3,
                    },
                    CanvasNode {
                        note_id: lin_note.id.clone(),
                        x: 870.0,
                        y: 70.0,
                        width: 300.0,
                        height: 180.0,
                        z_index: 1,
                    },
                    CanvasNode {
                        note_id: evidence_note.id.clone(),
                        x: 870.0,
                        y: 340.0,
                        width: 300.0,
                        height: 180.0,
                        z_index: 2,
                    },
                    CanvasNode {
                        note_id: risk_note.id.clone(),
                        x: 450.0,
                        y: 610.0,
                        width: 340.0,
                        height: 190.0,
                        z_index: 5,
                    },
                ],
                is_pinned: true,
                pin_position: Some(0),
            },
        )?
        .canvas;
    let canvas_view = db.canvas_get(&canvas.id)?;

    let view_props = vec![
        ViewProperty::InboxState,
        ViewProperty::Tags,
        ViewProperty::Archived,
        ViewProperty::UpdatedAt,
    ];
    let view_board = db
        .view_save(
            &id(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: "待整理看板".into(),
                filter_schema: 1,
                filter: NotesFilter {
                    inbox_state: Some(InboxState::Pending),
                    ..Default::default()
                },
                sort: NoteSort::UpdatedDesc,
                layout: ViewLayout::Board,
                config_schema: 1,
                config: ViewConfig {
                    visible_properties: view_props.clone(),
                    board_group_by: BoardGroupBy::InboxState,
                    calendar_month: Some("2026-10".into()),
                    custom_properties: vec![],
                    date_property: None,
                },
                is_pinned: true,
                pin_position: Some(0),
            },
        )?
        .view;
    let view_calendar = db
        .view_save(
            &id(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: "控制板故障日历".into(),
                filter_schema: 1,
                filter: NotesFilter {
                    collection_id: Some(project.id.clone()),
                    ..Default::default()
                },
                sort: NoteSort::UpdatedAsc,
                layout: ViewLayout::Calendar,
                config_schema: 1,
                config: ViewConfig {
                    visible_properties: view_props.clone(),
                    board_group_by: BoardGroupBy::InboxState,
                    calendar_month: Some("2026-10".into()),
                    custom_properties: vec![
                        "date".into(),
                        "severity".into(),
                        "review_phase".into(),
                    ],
                    date_property: Some("date".into()),
                },
                is_pinned: true,
                pin_position: Some(1),
            },
        )?
        .view;

    let view_timeline = db
        .view_save(
            &id(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: "项目复盘时间线".into(),
                filter_schema: 1,
                filter: NotesFilter {
                    collection_id: Some(project.id.clone()),
                    ..Default::default()
                },
                sort: NoteSort::UpdatedAsc,
                layout: ViewLayout::Timeline,
                config_schema: 1,
                config: ViewConfig {
                    visible_properties: view_props.clone(),
                    board_group_by: BoardGroupBy::Tag,
                    calendar_month: Some("2026-10".into()),
                    custom_properties: vec!["date".into(), "review_phase".into()],
                    date_property: Some("date".into()),
                },
                is_pinned: false,
                pin_position: None,
            },
        )?
        .view;
    let view_table = db
        .view_save(
            &id(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: "故障案例类型表".into(),
                filter_schema: 1,
                filter: NotesFilter {
                    object_types: vec!["fault-case".into()],
                    ..Default::default()
                },
                sort: NoteSort::TitleAsc,
                layout: ViewLayout::Table,
                config_schema: 1,
                config: ViewConfig {
                    visible_properties: view_props,
                    board_group_by: BoardGroupBy::InboxState,
                    calendar_month: Some("2026-10".into()),
                    custom_properties: vec!["date".into(), "severity".into()],
                    date_property: Some("date".into()),
                },
                is_pinned: true,
                pin_position: Some(2),
            },
        )?
        .view;
    let view_runs = [
        (
            view_board.id.clone(),
            db.view_run(&view_board.id, 50, None)?.notes.items.len(),
        ),
        (
            view_calendar.id.clone(),
            db.view_run(&view_calendar.id, 50, None)?.notes.items.len(),
        ),
        (
            view_timeline.id.clone(),
            db.view_run(&view_timeline.id, 50, None)?.notes.items.len(),
        ),
        (
            view_table.id.clone(),
            db.view_run(&view_table.id, 50, None)?.notes.items.len(),
        ),
    ];

    let review_repair = add_review(&mut db, &can_note.id)?;
    let review_interview = add_review(&mut db, &lin_note.id)?;
    let review_source = add_review(&mut db, &pdf_source.id)?;
    db.snooze_review(
        &id(),
        SnoozeReview {
            entry_id: review_source.id.clone(),
            base_row_version: review_source.row_version,
            due_at: (Utc::now() + Duration::days(14)).to_rfc3339(),
        },
    )?;
    let review_counts = db.review_counts(None)?;
    let settings = db.review_insight_settings()?;
    db.set_review_insight_settings(
        &id(),
        SetReviewInsightSettings {
            base_row_version: settings.row_version,
            enabled: true,
            window_days: 30,
            include_completed: true,
            include_stale: true,
        },
    )?;
    let insights = db.review_insights(20)?;

    let first_change = db
        .create_change_set(
            &id(),
            CreateChangeSet {
                kind: ChangeSetKind::TagSuggestions,
                collection_id: None,
                limit: 20,
                stale_after_days: None,
            },
        )?
        .change;
    let applied_change = if let Some(item) = first_change
        .items
        .iter()
        .find(|item| {
            matches!(item.payload, ChangePayload::AssignTag { .. })
                && item.target_object_id != main_note.id
        })
        .or_else(|| {
            first_change
                .items
                .iter()
                .find(|item| matches!(item.payload, ChangePayload::AssignTag { .. }))
        }) {
        Some(db.apply_change_item(
            &id(),
            ApplyChangeItem {
                change_id: first_change.id.clone(),
                item_id: item.item_id.clone(),
                confirm: false,
            },
        )?)
    } else {
        None
    };
    let pending_tag_change = db
        .create_change_set(
            &id(),
            CreateChangeSet {
                kind: ChangeSetKind::TagSuggestions,
                collection_id: None,
                limit: 20,
                stale_after_days: None,
            },
        )?
        .change;
    let relation_change = db
        .create_change_set(
            &id(),
            CreateChangeSet {
                kind: ChangeSetKind::RelationSuggestions,
                collection_id: None,
                limit: 8,
                stale_after_days: None,
            },
        )?
        .change;
    let audit_page = db.audit_page(pkms_core::AuditPageRequest {
        limit: 20,
        before_seq: None,
        subject_kind: None,
        subject_id: None,
    })?;

    trash_object(&mut db, &waste_source.id)?;

    let queries = [
        "不要先怪总线",
        "每小时 14 个错误帧",
        "先固定时间基准",
        "第 128 页",
        "看门狗复位",
        "连续覆盖 6 小时",
    ]
    .into_iter()
    .map(|value| knowledge_query(&db, value))
    .collect::<Result<Vec<_>>>()?;

    let graph = db.knowledge_graph()?;
    let trash = db.trash_page(pkms_core::TrashPageRequest {
        limit: 50,
        cursor: None,
        query: None,
    })?;

    fs::create_dir_all(backup_path.parent().ok_or("backup path has no parent")?)?;
    let mut check = |_stage: &str| -> pkms_core::Result<()> { Ok(()) };
    let backup = db.create_backup(backup_path, &mut check)?;

    let backup_bytes = backup.manifest.database.size
        + backup
            .manifest
            .blobs
            .iter()
            .map(|file| file.size)
            .sum::<u64>();
    let result = json!({
        "workspace": workspace,
        "workspace_id": workspace_id,
        "fixtures": fixtures,
        "collections": {
            "embedded": embedded.id,
            "can": can.id,
            "spi": spi.id,
            "rtos": rtos.id,
            "methods": methods.id,
            "people": people.id,
            "reading": reading.id,
            "project": project.id
        },
        "notes": {
            "main": main_note.id,
            "can": can_note.id,
            "spi": spi_note.id,
            "rtos": rtos_note.id,
            "lin_interview": lin_note.id,
            "zhou_interview": zhou_note.id,
            "evidence": evidence_note.id,
            "risk": risk_note.id,
            "meeting": meeting_note.id,
            "inbox": inbox_note.id,
            "guide": guide_note.id,
            "later": later_note.id,
            "trashed": trashed_note.id,
            "source_note": source_note.id
        },
        "sources": {
            "pdf": pdf_source.id,
            "manual": manual_source.id,
            "timeline": timeline_source.id,
            "dashboard": dashboard_source.id,
            "minutes": minutes_source.id,
            "csv": csv_source.id,
            "xlsx": xlsx_source.id,
            "pptx": pptx_source.id,
            "supplement": supplement_source.id,
            "waste": waste_source.id
        },
        "annotations": {
            "pdf": pdf_annotation.id,
            "manual": manual_annotation.id,
            "timeline": timeline_annotation.id,
            "dashboard": dashboard_annotation.id,
            "minutes": minutes_annotation.id,
            "csv": csv_annotation.id,
            "xlsx": xlsx_annotation.id,
            "pptx": pptx_annotation.id
        },
        "canvas": {
            "id": canvas_view.canvas.id,
            "nodes": canvas_view.canvas.nodes.len(),
            "resolved_notes": canvas_view.notes.len(),
            "missing": canvas_view.missing_note_ids.len()
        },
        "views": view_runs,
        "reviews": {
            "repair": review_repair.id,
            "interview": review_interview.id,
            "source": review_source.id,
            "counts": review_counts,
            "insight_count": insights.items.len()
        },
        "automation": {
            "tag_change": first_change.id,
            "tag_items": first_change.items.len(),
            "pending_tag_change": pending_tag_change.id,
            "pending_tag_items": pending_tag_change.items.len(),
            "relation_change": relation_change.id,
            "relation_items": relation_change.items.len(),
            "applied_item": applied_change.as_ref().map(|value| value.item.item_id.clone()),
            "audit_id": applied_change.as_ref().and_then(|value| value.audit.as_ref().map(|audit| audit.id.clone())),
            "audit_count": audit_page.items.len()
        },
        "graph": {
            "nodes": graph.nodes.len(),
            "edges": graph.edges.len(),
            "reference_edges": graph.edges.iter().filter(|edge| matches!(edge.kind, pkms_core::KnowledgeGraphEdgeKind::Reference)).count(),
            "relation_edges": graph.edges.iter().filter(|edge| matches!(edge.kind, pkms_core::KnowledgeGraphEdgeKind::Relation)).count(),
            "block_edges": graph.edges.iter().filter(|edge| matches!(edge.kind, pkms_core::KnowledgeGraphEdgeKind::BlockReference)).count(),
            "canvas_edges": graph.edges.iter().filter(|edge| matches!(edge.kind, pkms_core::KnowledgeGraphEdgeKind::Canvas)).count()
        },
        "trash": {
            "items": trash.items.len(),
            "total": trash.total
        },
        "pdf_pages": pdf_parsed.page_count,
        "backup": {
            "path": backup_path,
            "files": backup.manifest.blobs.len() + 1,
            "bytes": backup_bytes,
            "verified_revisions": backup.verified_revisions,
            "verified_blobs": backup.verified_blobs
        },
        "queries": queries
    });
    fs::write(report_path, serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

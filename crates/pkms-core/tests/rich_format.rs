//! D3/K15 increment: real persistence and archives, without remote model calls.
use pkms_core::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Cursor, Read},
    path::Path,
};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn document(mut blocks: Value) -> NoteDocument {
    fn identities(node: &mut Value) {
        if !matches!(
            node["type"].as_str().unwrap(),
            "text"
                | "hardBreak"
                | "noteReference"
                | "inlineMath"
                | "wikiLink"
                | "footnoteReference"
                | "comment"
        ) {
            if node.get("attrs").is_none() {
                node["attrs"] = json!({});
            }
            if node["attrs"].get("id").is_none() {
                node["attrs"]["id"] = json!(id());
            }
        }
        if matches!(
            node["type"].as_str().unwrap(),
            "noteReference" | "wikiLink" | "wikiEmbed"
        ) && node["attrs"].get("occurrence_id").is_none()
        {
            node["attrs"]["occurrence_id"] = json!(id());
        }
        if let Some(children) = node.get_mut("content").and_then(Value::as_array_mut) {
            children.iter_mut().for_each(identities);
        }
    }
    blocks
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .for_each(identities);
    serde_json::from_value(json!({"type":"doc","content":blocks})).unwrap()
}

fn all_formats() -> NoteDocument {
    document(json!([
        {"type":"frontmatter","attrs":{"source":"title: HiddenPropertiesOnly\nnested:\n  unknown: [1, true]\n"}},
        {"type":"heading","attrs":{"level":6,"block_id":"chapter-six"},"content":[{"type":"text","text":"六级标题"}]},
        {"type":"paragraph","content":[
            {"type":"text","text":"格式组合","marks":[{"type":"bold"},{"type":"italic"},{"type":"strike"},{"type":"underline"},{"type":"highlight","attrs":{"color":"#FFE08A"}},{"type":"textStyle","attrs":{"color":"#1234Ab"}}]},
            {"type":"text","text":"上","marks":[{"type":"superscript"}]},
            {"type":"text","text":"下","marks":[{"type":"subscript"}]},
            {"type":"text","text":"黄色","marks":[{"type":"highlight"}]},
            {"type":"inlineMath","attrs":{"source":"E = mc^2"},"marks":[{"type":"underline"}]},
            {"type":"wikiLink","attrs":{"wiki_target":"尚未创建#标题","label":"内部引用"},"marks":[{"type":"italic"}]},
            {"type":"footnoteReference","attrs":{"label":"second"}},
            {"type":"footnoteReference","attrs":{"label":"FIRST"}},
            {"type":"comment","attrs":{"source":"HiddenCommentOnly"}}
        ]},
        {"type":"callout","attrs":{"callout_type":"custom-note","title":"提示 & <safe>","collapsed":false},"content":[
            {"type":"paragraph","content":[{"type":"text","text":"CalloutVisibleOnly"}]},
            {"type":"callout","attrs":{"callout_type":"warning","collapsed":true},"content":[]}
        ]},
        {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"无序项目"}]}]}]},
        {"type":"orderedList","attrs":{"start":3},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"有序项目"}]}]}]},
        {"type":"taskList","content":[{"type":"taskItem","attrs":{"checked":true,"task_marker":"!"},"content":[{"type":"paragraph","content":[{"type":"text","text":"自定义任务"}]}]}]},
        {"type":"table","content":[
            {"type":"tableRow","content":[{"type":"tableHeader","attrs":{"align":"left"},"content":[{"type":"paragraph","content":[{"type":"text","text":"左"}]}]},{"type":"tableHeader","attrs":{"align":"right"},"content":[{"type":"paragraph","content":[{"type":"text","text":"右"}]}]}]},
            {"type":"tableRow","content":[{"type":"tableCell","attrs":{"align":"left"},"content":[{"type":"paragraph","content":[{"type":"text","text":"a|b"}]}]},{"type":"tableCell","attrs":{"align":"right"},"content":[{"type":"paragraph","content":[{"type":"text","text":"c"}]}]}]}
        ]},
        {"type":"blockMath","attrs":{"source":"\\sum_{i=1}^{n} i"}},
        {"type":"codeBlock","attrs":{"language":"mermaid"},"content":[{"type":"text","text":"graph LR\n  A[开始] --> B[结束]"}]},
        {"type":"wikiEmbed","attrs":{"wiki_target":"材料.pdf","label":"材料","embed_width":640,"embed_height":480}},
        {"type":"footnoteDefinition","attrs":{"label":"first"},"content":[{"type":"paragraph","content":[{"type":"text","text":"第一个定义"}]}]},
        {"type":"footnoteDefinition","attrs":{"label":"second"},"content":[{"type":"paragraph","content":[{"type":"text","text":"第二个定义"}]},{"type":"paragraph","content":[{"type":"text","text":"第二段脚注"}]}]},
        {"type":"commentBlock","attrs":{"source":"HiddenBlockOnly\n保留换行"}}
    ]))
}

fn create(title: &str, document: NoteDocument) -> CreateNote {
    CreateNote {
        title: title.into(),
        document_schema: 3,
        document,
        collection_id: None,
        base_collection_row_version: None,
    }
}

fn unpack(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut file = archive.by_index(index).unwrap();
            let name = file.name().to_owned();
            let mut bytes = vec![];
            file.read_to_end(&mut bytes).unwrap();
            (name, bytes)
        })
        .collect()
}

#[test]
fn rich_format_schema_three_preserves_every_format_and_legacy_serialization() {
    let rich = all_formats();
    rich.validate(3).unwrap();
    assert!(rich.validate(1).is_err());
    assert!(rich.validate(2).is_err());
    assert!(rich.validate(4).is_err());
    assert_eq!(
        serde_json::from_slice::<NoteDocument>(&serde_json::to_vec(&rich).unwrap()).unwrap(),
        rich
    );
    let legacy = r#"{"type":"doc","content":[{"type":"paragraph","attrs":{"id":"11111111-1111-4111-8111-111111111111"},"content":[{"type":"text","text":"旧格式","marks":[{"type":"bold"}]}]}]}"#;
    let old: NoteDocument = serde_json::from_str(legacy).unwrap();
    old.validate(2).unwrap();
    assert_eq!(serde_json::to_string(&old).unwrap(), legacy);
    for kind in [
        "underline",
        "superscript",
        "subscript",
        "highlight",
        "textStyle",
    ] {
        let mark = if kind == "textStyle" {
            json!({"type":kind,"attrs":{"color":"#ABC123"}})
        } else {
            json!({"type":kind})
        };
        let doc = document(
            json!([{"type":"paragraph","content":[{"type":"text","text":"新样式","marks":[mark]}]}]),
        );
        doc.validate(3).unwrap();
        assert!(doc.validate(2).is_err(), "{kind}");
    }
    for block in &rich.content {
        let doc = NoteDocument {
            kind: DocumentType::Doc,
            content: vec![block.clone()],
        };
        if matches!(
            block.kind,
            NodeType::BulletList | NodeType::OrderedList | NodeType::CodeBlock
        ) {
            doc.validate(2).unwrap();
            continue;
        }
        assert!(doc.validate(2).is_err(), "{:?}", block.kind);
    }
}

#[test]
fn rich_format_rejects_unsafe_attributes_bad_combinations_and_unbounded_sources() {
    let invalid = vec![
        json!([{"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"highlight","attrs":{"color":"red;background:url(x)"}}]}]}]),
        json!([{"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"superscript"},{"type":"subscript"}]}]}]),
        json!([{"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"code"},{"type":"underline"}]}]}]),
        json!([{"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"textStyle"}]}]}]),
        json!([{"type":"heading","attrs":{"level":7}}]),
        json!([{"type":"blockMath","attrs":{"source":"  "}}]),
        json!([{"type":"blockMath","attrs":{"source":"中".repeat(21_846)}}]),
        json!([{"type":"commentBlock","attrs":{"source":"x\u{0000}y"}}]),
        json!([{"type":"callout","attrs":{"callout_type":"note\" onload=bad"}}]),
        json!([{"type":"wikiEmbed","attrs":{"wiki_target":"材料","embed_width":4097}}]),
        json!([{"type":"wikiEmbed","attrs":{"wiki_target":"材料","embed_height":0}}]),
        json!([{"type":"paragraph","content":[{"type":"wikiLink","attrs":{"wiki_target":"材料","target_block_id":id()}}]}]),
        json!([{"type":"paragraph","attrs":{"block_id":"same"}},{"type":"paragraph","attrs":{"block_id":"same"}}]),
        json!([{"type":"paragraph","attrs":{"block_id":"not_valid"}}]),
        json!([{"type":"paragraph"},{"type":"frontmatter","attrs":{"source":""}}]),
        json!([{"type":"blockquote","content":[{"type":"frontmatter","attrs":{"source":""}}]}]),
        json!([{"type":"footnoteDefinition","attrs":{"label":"Mixed  Case"}},{"type":"footnoteDefinition","attrs":{"label":" mixed case "}}]),
        json!([{"type":"taskList","content":[{"type":"taskItem","attrs":{"checked":false,"task_marker":"[]"},"content":[{"type":"paragraph"}]}]}]),
    ];
    for (index, blocks) in invalid.into_iter().enumerate() {
        assert!(
            document(blocks).validate(3).is_err(),
            "accepted invalid case {index}"
        );
    }
    let boundary = document(json!([{"type":"commentBlock","attrs":{"source":"x".repeat(65_536)}}]));
    boundary.validate(3).unwrap();
    let mut changed = serde_json::to_value(boundary).unwrap();
    changed["content"][0]["attrs"]["onclick"] = json!("bad");
    assert!(serde_json::from_value::<NoteDocument>(changed).is_err());
}

#[test]
fn rich_format_revision_copy_history_and_backup_keep_data_and_hide_private_text_from_search() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let first = db
        .create_document(&id(), create("富格式", all_formats()))
        .unwrap()
        .note;
    for hidden in [
        "HiddenPropertiesOnly",
        "HiddenCommentOnly",
        "HiddenBlockOnly",
    ] {
        assert!(db.search(hidden, 10).unwrap().is_empty(), "{hidden}");
    }
    assert_eq!(db.search("CalloutVisibleOnly", 10).unwrap().len(), 1);
    let copied = db
        .copy_document(
            &id(),
            CopyNote {
                note_id: first.id.clone(),
                source_revision_id: first.revision_id.clone(),
                title: "副本".into(),
            },
        )
        .unwrap()
        .note;
    assert_eq!(copied.schema_version, 3);
    assert_eq!(copied.document.plain_text(), first.document.plain_text());
    assert_ne!(
        copied.document.content[1].attrs.id,
        first.document.content[1].attrs.id
    );
    assert_eq!(
        copied.document.content[1].attrs.block_id,
        first.document.content[1].attrs.block_id
    );
    assert_eq!(
        copied.document.content[2].content[0].marks,
        first.document.content[2].content[0].marks
    );
    let changed = db
        .save_document(
            &id(),
            SaveNote {
                note_id: first.id.clone(),
                base_revision_id: first.revision_id.clone(),
                title: first.title.clone(),
                document_schema: 3,
                document: NoteDocument::from_text(id(), "改后内容"),
            },
        )
        .unwrap()
        .note;
    let restored = db
        .restore_document(
            &id(),
            RestoreNote {
                note_id: first.id.clone(),
                base_revision_id: changed.revision_id,
                target_revision_id: first.revision_id.clone(),
            },
        )
        .unwrap()
        .note;
    assert_eq!(restored.document, first.document);
    assert_eq!(restored.content_hash, first.content_hash);
    let package = root.path().join("rich.pkmsbackup");
    let report = db.create_backup(&package, &mut |_| Ok(())).unwrap();
    assert_eq!(
        report.manifest.database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    assert_eq!(report.manifest.document_schemas, [1, 2, 3]);
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let other = Library::open_existing(destination).unwrap();
    assert_eq!(other.note(&first.id).unwrap(), restored);
    assert_eq!(other.note(&copied.id).unwrap(), copied);
    assert_eq!(
        other
            .revision(&first.id, &first.revision_id)
            .unwrap()
            .content_hash,
        first.content_hash
    );
}

#[test]
fn rich_format_wiki_binds_unique_targets_and_anchors_without_rebinding_or_guessing_paths() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let target = db.create_document(&id(), create("目录/目标", document(json!([
        {"type":"heading","attrs":{"level":1},"content":[{"type":"text","text":"章"}]},
        {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"节"}]},
        {"type":"paragraph","attrs":{"block_id":"block-one"},"content":[{"type":"text","text":"原块"}]}
    ])))).unwrap().note;
    db.create_note("重复", "one").unwrap();
    db.create_note("重复", "two").unwrap();
    let request = create(
        "目录/引用",
        document(json!([
            {"type":"heading","attrs":{"level":2,"block_id":"self-one"},"content":[{"type":"text","text":"自己"}]},
            {"type":"paragraph","content":[
                {"type":"wikiLink","attrs":{"wiki_target":"./目标.md#章#节","label":"层级标题"}},
                {"type":"wikiLink","attrs":{"wiki_target":"目录/目标#^block-one"}},
                {"type":"wikiLink","attrs":{"wiki_target":"#^self-one"}},
                {"type":"wikiLink","attrs":{"wiki_target":"重复"}},
                {"type":"wikiLink","attrs":{"wiki_target":"另一个目录/目标"}},
                {"type":"wikiLink","attrs":{"wiki_target":"不存在"}}
            ]},
            {"type":"wikiEmbed","attrs":{"wiki_target":"目录/目标"}}
        ])),
    );
    let command = id();
    let first = db.create_document(&command, request.clone()).unwrap().note;
    let refs = &first.document.content[1].content;
    assert_eq!(refs[0].attrs.target_id.as_deref(), Some(target.id.as_str()));
    assert_eq!(
        refs[0].attrs.target_block_id.as_deref(),
        Some(target.document.content[1].attrs.id.as_str())
    );
    assert_eq!(
        refs[0].attrs.wiki_target,
        request.document.content[1].content[0].attrs.wiki_target
    );
    assert_eq!(
        refs[1].attrs.target_block_id.as_deref(),
        Some(target.document.content[2].attrs.id.as_str())
    );
    assert_eq!(refs[2].attrs.target_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(
        refs[2].attrs.target_block_id.as_deref(),
        Some(first.document.content[0].attrs.id.as_str())
    );
    assert!(refs[3..].iter().all(|node| node.attrs.target_id.is_none()));
    assert_eq!(db.body_backlinks(&target.id).unwrap().len(), 3);
    let outgoing = db.note_references(&first.id, &first.revision_id).unwrap();
    assert_eq!(outgoing.len(), 4);
    assert_eq!(
        outgoing[3].source_block_id,
        first.document.content[2].attrs.id
    );
    let renamed = db
        .save_document(
            &id(),
            SaveNote {
                note_id: target.id.clone(),
                base_revision_id: target.revision_id,
                title: "已改名".into(),
                document_schema: 3,
                document: target.document.clone(),
            },
        )
        .unwrap()
        .note;
    db.create_note("目录/目标", "新的同名目标").unwrap();
    assert_eq!(db.create_document(&command, request).unwrap().note, first);
    let second = db
        .save_document(
            &id(),
            SaveNote {
                note_id: first.id.clone(),
                base_revision_id: first.revision_id.clone(),
                title: first.title.clone(),
                document_schema: 3,
                document: first.document.clone(),
            },
        )
        .unwrap()
        .note;
    assert_eq!(second.document, first.document);
    assert_eq!(
        db.resolve_link(ResolveLink {
            target_id: renamed.id.clone(),
            target_block_id: refs[0].attrs.target_block_id.clone(),
            target_revision_id: None
        })
        .unwrap()
        .state,
        LinkState::Available
    );
    let copied = db
        .copy_document(
            &id(),
            CopyNote {
                note_id: first.id.clone(),
                source_revision_id: second.revision_id.clone(),
                title: "副本".into(),
            },
        )
        .unwrap()
        .note;
    assert_eq!(
        copied.document.content[1].content[2]
            .attrs
            .target_id
            .as_deref(),
        Some(copied.id.as_str())
    );
    assert_eq!(
        copied.document.content[1].content[0]
            .attrs
            .target_id
            .as_deref(),
        Some(target.id.as_str())
    );
    let archived = root.path().join("wiki.pkmsbackup");
    db.create_backup(&archived, &mut |_| Ok(())).unwrap();
}

#[test]
fn rich_format_markdown_and_html_export_preserve_real_syntax_numbered_footnotes_and_escaping() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let mut doc = all_formats();
    doc.content[2].content.push(
        serde_json::from_value(json!({"type":"text","text":"<script>alert(1)</script>"})).unwrap(),
    );
    let note = db
        .create_document(&id(), create("安全 <title>", doc))
        .unwrap()
        .note;
    let markdown = root.path().join("markdown.zip");
    let report = db
        .export_note_archive(&note.id, &markdown, ExportFormat::Markdown, &mut |_| Ok(()))
        .unwrap();
    let files = unpack(&markdown);
    let source = String::from_utf8(files[&report.document_path].clone()).unwrap();
    assert!(source.starts_with(
        "---\ntitle: HiddenPropertiesOnly\nnested:\n  unknown: [1, true]\n\n---\n\n#"
    ));
    for syntax in [
        "###### 六级标题 ^chapter-six",
        "==黄色==",
        "<u>",
        "background-color:#FFE08A",
        "color:#1234Ab",
        "<sup>上</sup>",
        "<sub>下</sub>",
        "<u>$E = mc^2$</u>",
        "- 无序项目",
        "3. 有序项目",
        "- [!] 自定义任务",
        "| :--- | ---: |",
        "a\\|b",
        "> [!custom-note]+",
        "> > [!warning]-",
        "```mermaid\n",
        "![[材料.pdf|材料|640x480]]",
        "[^second]",
        "[^first]:",
        "%%HiddenBlockOnly\n保留换行%%",
    ] {
        assert!(source.contains(syntax), "missing {syntax:?} in {source}");
    }
    assert!(!source.contains("<script>"));
    let html = root.path().join("html.zip");
    let report = db
        .export_note_archive(&note.id, &html, ExportFormat::Html, &mut |_| Ok(()))
        .unwrap();
    let files = unpack(&html);
    for asset in [
        "assets/rich-export/renderer.js",
        "assets/rich-export/style.css",
        "assets/rich-export/manifest.json",
    ] {
        assert!(
            files.contains_key(asset),
            "missing local renderer resource {asset}"
        );
    }
    let rendered = String::from_utf8(files[&report.document_path].clone()).unwrap();
    for syntax in [
        "data-pkms-math=\"inline\"",
        "data-pkms-math=\"block\"",
        "data-pkms-mermaid",
        "hidden data-pkms-comment",
        "text-align:right",
        "href=\"#pkms-footnote-1\">1</a>",
        "href=\"#pkms-footnote-2\">2</a>",
        "<u><span data-pkms-math",
        "&lt;script&gt;alert(1)&lt;/script&gt;",
        "<details",
        " data-callout=\"custom-note\" open",
        "data-task-marker=\"!\"",
        "class=\"pkms-tasks\"",
    ] {
        assert!(
            rendered.contains(syntax),
            "missing {syntax:?} in {rendered}"
        );
    }
    assert!(!rendered.contains("<script>alert"));
    assert!(rendered.contains("default-src 'none'"));
    assert!(rendered.contains("<script defer src=\"assets/rich-export/renderer.js\"></script>"));
    assert!(rendered.contains("font-src 'self' data:"));
}

#[test]
fn docx_export_preserves_table_alignment_and_task_state() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let note = db
        .create_document(&id(), create("DOCX 版式", all_formats()))
        .unwrap()
        .note;
    let path = root.path().join("layout.docx");
    db.export_note_archive(&note.id, &path, ExportFormat::Docx, &mut |_| Ok(()))
        .unwrap();
    let files = unpack(&path);
    let document = String::from_utf8(files["word/document.xml"].clone()).unwrap();
    assert!(
        document.contains("<w:tblW w:w=\"5000\" w:type=\"pct\"/>"),
        "{document}"
    );
    assert!(document.contains("<w:jc w:val=\"right\"/>"), "{document}");
    assert!(document.contains("<w:jc w:val=\"left\"/>"), "{document}");
    assert!(document.contains("☑"), "{document}");
    assert!(!document.contains("☐"), "only the checked task exists");
}

#[test]
fn rich_format_markdown_rejects_unrepresentable_values_without_publishing_a_file() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    for (index, value) in [
        json!([{"type":"commentBlock","attrs":{"source":"contains %% delimiter"}}]),
        json!([{"type":"commentBlock","attrs":{"source":"trailing\\"}}]),
        json!([{"type":"frontmatter","attrs":{"source":"value: before\n---\nafter: value"}}]),
        json!([{"type":"wikiEmbed","attrs":{"wiki_target":"材料","embed_height":100}}]),
        json!([{"type":"wikiEmbed","attrs":{"wiki_target":"材料","label":"640"}}]),
        json!([{"type":"taskList","content":[{"type":"taskItem","attrs":{"checked":false,"task_marker":"!"},"content":[{"type":"paragraph"}]}]}]),
        json!([{"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph"}]}]}]}]),
    ].into_iter().enumerate() {
        let note = db.create_document(&id(), create("可存但无标准MD表示", document(value))).unwrap().note;
        let destination = root.path().join(format!("rejected-{index}.zip"));
        assert!(matches!(db.export_note_archive(&note.id, &destination, ExportFormat::Markdown, &mut |_| Ok(())), Err(Error::InvalidInput(_))));
        assert!(!destination.exists());
        db.export_note_archive(&note.id, &destination, ExportFormat::Html, &mut |_| Ok(())).unwrap();
    }
}

fn capture(db: &mut Library, name: &str) -> SourceVersion {
    let accepted = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: name.into(),
                format: SourceFormat::Markdown,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(format!("# {name}")),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.cancel_source_job(&accepted.job_id).unwrap();
    version
}

#[test]
fn rich_format_source_import_accepts_old_intents_and_binds_new_wiki_without_changing_original() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let target = db.create_note("目标", "可引用").unwrap();
    let source = capture(&mut db, "资料.md");
    let legacy = CreateSourceNote {
        source_version_id: source.id.clone(),
        source_blob_hash: source.blob_hash.clone(),
        title: "旧待办".into(),
        document_schema: 2,
        document: NoteDocument::from_text(id(), "旧待办正文"),
    };
    let command = id();
    let old = db
        .create_source_note(&command, legacy.clone())
        .unwrap()
        .note;
    assert_eq!(old.schema_version, 2);
    let request = CreateSourceNote {
        document_schema: 3,
        document: document(json!([
            {"type":"paragraph","content":[{"type":"text","text":"新高亮","marks":[{"type":"highlight"}]},{"type":"wikiLink","attrs":{"wiki_target":"目标"}}]},
            {"type":"wikiEmbed","attrs":{"wiki_target":"资料.md"}}
        ])),
        title: "新导入".into(),
        ..legacy.clone()
    };
    let new = db.create_source_note(&id(), request).unwrap().note;
    assert_eq!(
        new.document.content[0].content[1]
            .attrs
            .target_id
            .as_deref(),
        Some(target.id.as_str())
    );
    assert_eq!(
        new.document.content[1].attrs.target_id.as_deref(),
        Some(source.source_id.as_str())
    );
    assert_eq!(db.source_note_input(&source.id).unwrap().text, "# 资料.md");
    assert_eq!(db.create_source_note(&command, legacy).unwrap().note, old);
}

fn completed_origin(db: &mut Library) -> String {
    let evidence = db.create_note("验收证据", "fixtureevidence").unwrap();
    let answer = db
        .accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                agent: None,
                search: SearchQuery {
                    query: "fixtureevidence".into(),
                    mode: SearchMode::Lexical,
                    scope: SearchScope {
                        object_ids: vec![evidence.id],
                        ..Default::default()
                    },
                    limit: 1,
                    minimum_cosine: None,
                },
                model: Some(GenerationProfile {
                    channel_id: None,
                    capabilities: None,
                    chat_token_limit: None,
                    provider: "ollama".into(),
                    endpoint: "http://127.0.0.1:11434".into(),
                    model: "controlled".into(),
                    model_digest: "controlled".into(),
                    config_revision: 1,
                    temperature: 0.0,
                    context_tokens: 4096,
                }),
            },
        )
        .unwrap()
        .answer;
    let lease = db.claim_answer(&answer.id).unwrap().unwrap();
    db.prepare_answer_evidence(&answer.id, &lease.lease_token, vec![])
        .unwrap();
    db.complete_answer(CompleteAnswer {
        answer_id: answer.id.clone(),
        lease_token: lease.lease_token,
        text: "Controlled fixture [E1]".into(),
        cited_ids: vec!["E1".into()],
    })
    .unwrap();
    answer.id
}

#[test]
fn rich_format_proposal_freezes_wiki_binding_before_review_and_applies_exact_reviewed_document() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let external = db.create_note("外部目标", "外部原文").unwrap();
    let target = db.create_note("正在编辑", "修改前").unwrap();
    let answer = completed_origin(&mut db);
    let proposed = document(json!([
        {"type":"heading","attrs":{"level":5,"block_id":"topic"},"content":[{"type":"text","text":"提案标题"}]},
        {"type":"paragraph","content":[{"type":"text","text":"颜色保留","marks":[{"type":"textStyle","attrs":{"color":"#123456"}},{"type":"underline"}]},{"type":"wikiLink","attrs":{"wiki_target":"外部目标"}},{"type":"wikiLink","attrs":{"wiki_target":"#^topic"}}]}
    ]));
    let request = CreateProposal {
        answer_id: Some(answer.clone()),
        chat_run_id: None,
        external_request_id: None,
        note_id: Some(target.id.clone()),
        base_revision_id: Some(target.revision_id),
        title: "待审富格式".into(),
        document_schema: 3,
        document: proposed.clone(),
    };
    let command = id();
    let pending = db
        .create_proposal(&command, request.clone())
        .unwrap()
        .proposal;
    assert!(
        request.document.content[1].content[1]
            .attrs
            .target_id
            .is_none()
    );
    assert_eq!(
        pending.request.document.content[1].content[1]
            .attrs
            .target_id
            .as_deref(),
        Some(external.id.as_str())
    );
    assert_eq!(
        pending.request.document.content[1].content[2]
            .attrs
            .target_id
            .as_deref(),
        Some(target.id.as_str())
    );
    db.save_document(
        &id(),
        SaveNote {
            note_id: external.id.clone(),
            base_revision_id: external.revision_id,
            title: "已改名外部目标".into(),
            document_schema: 3,
            document: external.document,
        },
    )
    .unwrap();
    db.create_note("外部目标", "不应重新绑定").unwrap();
    assert_eq!(
        db.create_proposal(&command, request)
            .unwrap()
            .proposal
            .request
            .document,
        pending.request.document
    );
    let applied = db.apply_proposal(&id(), &pending.id).unwrap().note;
    assert_eq!(applied.document, pending.request.document);
    let new_pending = db.create_proposal(&id(), CreateProposal { answer_id: Some(answer), chat_run_id: None, external_request_id: None, note_id: None, base_revision_id: None, title: "新提案".into(), document_schema: 3, document: document(json!([
        {"type":"heading","attrs":{"level":1,"block_id":"new-self"},"content":[{"type":"text","text":"新笔记"}]},
        {"type":"paragraph","content":[{"type":"wikiLink","attrs":{"wiki_target":"#^new-self"}}]}
    ])) }).unwrap().proposal;
    assert!(
        new_pending.request.document.content[1].content[0]
            .attrs
            .target_id
            .is_none()
    );
    let new_note = db.apply_proposal(&id(), &new_pending.id).unwrap().note;
    assert_eq!(new_note.document, new_pending.request.document);
    let package = root.path().join("proposals.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    verify_backup(package, &mut |_| Ok(())).unwrap();
}

#[test]
fn rich_format_nested_embed_backlink_uses_its_own_block_and_display_label() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let target = db.create_note("嵌入目标", "内容").unwrap();
    let note = db.create_document(&id(), create("嵌入者", document(json!([
        {"type":"callout","attrs":{"callout_type":"note"},"content":[{"type":"wikiEmbed","attrs":{"wiki_target":"嵌入目标"}}]}
    ])))).unwrap().note;
    let reference = db.body_backlinks(&target.id).unwrap().remove(0);
    assert_eq!(
        reference.source_block_id,
        note.document.content[0].content[0].attrs.id
    );
    assert_eq!(reference.label, "嵌入目标");
    assert_eq!(reference.context, "嵌入目标");
}

#[test]
fn rich_format_html_alias_does_not_shadow_an_internal_block_or_a_footnote_anchor() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let other_id = id();
    let note = db.create_document(&id(), create("同名锚点", document(json!([
        {"type":"paragraph","attrs":{"block_id":other_id},"content":[{"type":"text","text":"别名属于第一块"}]},
        {"type":"paragraph","attrs":{"id":other_id,"block_id":"pkms-footnote-1"},"content":[{"type":"footnoteReference","attrs":{"label":"fn"}}]},
        {"type":"footnoteDefinition","attrs":{"label":"fn"}}
    ])))).unwrap().note;
    let output = root.path().join("html.zip");
    let report = db
        .export_note_archive(&note.id, &output, ExportFormat::Html, &mut |_| Ok(()))
        .unwrap();
    let files = unpack(&output);
    let html = String::from_utf8(files[&report.document_path].clone()).unwrap();
    assert_eq!(html.matches(&format!("id=\"{other_id}\"")).count(), 1);
    assert_eq!(html.matches("id=\"pkms-footnote-1\"").count(), 1);
    assert!(html.contains(&format!("data-block-alias=\"{other_id}\"")));
    assert!(!files.contains_key("assets/rich-export/renderer.js"));
}

#[test]
fn rich_format_comments_and_properties_never_enter_embedding_input() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db
        .create_document(&id(), create("检索输入", all_formats()))
        .unwrap()
        .note;
    let input = db.index_input(&note.id).unwrap();
    let text = input
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for hidden in [
        "HiddenPropertiesOnly",
        "HiddenCommentOnly",
        "HiddenBlockOnly",
    ] {
        assert!(!text.contains(hidden));
    }
    assert!(text.contains("CalloutVisibleOnly"));
    assert!(text.contains("E = mc^2"));
}

#[test]
fn rich_format_markdown_comments_preserve_safe_boundaries_or_refuse_before_publish() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    for (index, source) in [
        "ends with %",
        "one\n\ntwo",
        "one\n# heading\ntwo",
        "one\n    indented",
        "one\r\n1. list",
        "\nmissing final fence newline",
    ]
    .into_iter()
    .enumerate()
    {
        let note = db
            .create_document(
                &id(),
                create(
                    "不可表示的注释",
                    document(json!([{"type":"commentBlock","attrs":{"source":source}}])),
                ),
            )
            .unwrap()
            .note;
        let output = root.path().join(format!("comment-{index}.zip"));
        assert!(matches!(
            db.export_note_archive(&note.id, &output, ExportFormat::Markdown, &mut |_| Ok(())),
            Err(Error::InvalidInput(_))
        ));
        assert!(!output.exists());
        db.export_note_archive(&note.id, &output, ExportFormat::Html, &mut |_| Ok(()))
            .unwrap();
    }
    for (index, source) in [
        "HiddenBlockOnly\n保留换行",
        "\none\n\n# literal heading\n- literal list\n> literal quote\n",
        "\r\n原样CRLF\r\n\r\n# literal heading\r\n",
    ]
    .into_iter()
    .enumerate()
    {
        let note = db
            .create_document(
                &id(),
                create(
                    "可表示的注释",
                    document(json!([{"type":"commentBlock","attrs":{"source":source}}])),
                ),
            )
            .unwrap()
            .note;
        let output = root.path().join(format!("safe-comment-{index}.zip"));
        let report = db
            .export_note_archive(&note.id, &output, ExportFormat::Markdown, &mut |_| Ok(()))
            .unwrap();
        let files = unpack(&output);
        let markdown = String::from_utf8(files[&report.document_path].clone()).unwrap();
        assert!(markdown.contains(&format!("%%{source}%%")));
    }
}

#[test]
fn rich_format_markdown_footnote_labels_encode_whitespace_once_without_changing_wiki_names() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let spaced = "A  B\u{2003}C & label";
    let literal_entity = "&#32;";
    let note = db.create_document(&id(), create("脚注标签", document(json!([
        {"type":"paragraph","content":[
            {"type":"wikiLink","attrs":{"wiki_target":"A & B"}},
            {"type":"footnoteReference","attrs":{"label":spaced}},
            {"type":"footnoteReference","attrs":{"label":literal_entity}}
        ]},
        {"type":"footnoteDefinition","attrs":{"label":spaced},"content":[{"type":"paragraph","content":[{"type":"text","text":"空白标签"}]}]},
        {"type":"footnoteDefinition","attrs":{"label":literal_entity},"content":[{"type":"paragraph","content":[{"type":"text","text":"字面实体"}]}]}
    ])))).unwrap().note;
    let output = root.path().join("footnotes.zip");
    let report = db
        .export_note_archive(&note.id, &output, ExportFormat::Markdown, &mut |_| Ok(()))
        .unwrap();
    let files = unpack(&output);
    let markdown = String::from_utf8(files[&report.document_path].clone()).unwrap();
    assert!(markdown.contains("[[A & B]]"));
    assert_eq!(
        markdown
            .matches("[^A&#32;&#32;B&#8195;C&#32;&#38;&#32;label]")
            .count(),
        2
    );
    assert_eq!(markdown.matches("[^&#38;#32;]").count(), 2);
    for (index, label) in ["[".repeat(512), "\u{1f4dd}".repeat(500)]
        .into_iter()
        .enumerate()
    {
        let long = db
            .create_document(
                &id(),
                create(
                    "长脚注标签",
                    document(json!([{"type":"footnoteDefinition","attrs":{"label":label}}])),
                ),
            )
            .unwrap()
            .note;
        let destination = root.path().join(format!("long-label-{index}.zip"));
        assert!(matches!(
            db.export_note_archive(&long.id, &destination, ExportFormat::Markdown, &mut |_| Ok(
                ()
            )),
            Err(Error::InvalidInput(_))
        ));
        assert!(!destination.exists());
        db.export_note_archive(&long.id, &destination, ExportFormat::Html, &mut |_| Ok(()))
            .unwrap();
    }
}

#[test]
fn rich_format_review_table_comments_keep_source_in_html_and_backup_but_refuse_multiline_markdown()
{
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let mut originals = vec![];
    for (index, source) in ["第一行\n第二行", "第一行\r\n第二行", "第一行\r第二行"]
        .into_iter()
        .enumerate()
    {
        let note = db.create_document(&id(), create("表格注释", document(json!([
            {"type":"table","content":[
                {"type":"tableRow","content":[{"type":"tableHeader","content":[{"type":"paragraph","content":[{"type":"text","text":"表头"}]}]}]},
                {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"comment","attrs":{"source":source}}]}]}]}
            ]}
        ])))).unwrap().note;
        let output = root.path().join(format!("table-{index}.zip"));
        assert!(matches!(
            db.export_note_archive(&note.id, &output, ExportFormat::Markdown, &mut |_| Ok(())),
            Err(Error::InvalidInput(
                "Markdown cannot preserve multiline comment sources inside a table; choose HTML or full backup"
            ))
        ));
        assert!(!output.exists());
        let report = db
            .export_note_archive(&note.id, &output, ExportFormat::Html, &mut |_| Ok(()))
            .unwrap();
        let files = unpack(&output);
        let html = String::from_utf8(files[&report.document_path].clone()).unwrap();
        assert!(html.contains(&format!(
            "hidden data-pkms-comment=\"true\">{source}</span>"
        )));
        originals.push(note);
    }
    let package = root.path().join("table.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(destination).unwrap();
    for original in originals {
        assert_eq!(restored.note(&original.id).unwrap(), original);
    }
}

#[test]
fn rich_format_review_html_wiki_missing_anchors_are_visible_without_false_links() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    db.create_note("外部目标", "没有锚点").unwrap();
    let original = db.create_document(&id(), create("HTML锚点", document(json!([
        {"type":"heading","attrs":{"level":2,"block_id":"live"},"content":[{"type":"text","text":"有效目标"}]},
        {"type":"paragraph","content":[
            {"type":"wikiLink","attrs":{"wiki_target":"#^missing","label":"缺失块"}},
            {"type":"wikiLink","attrs":{"wiki_target":"#不存在的标题","label":"缺失标题"}},
            {"type":"wikiLink","attrs":{"wiki_target":"%23%5Emissing-encoded","label":"编码锚"}},
            {"type":"wikiLink","attrs":{"wiki_target":"外部目标#^missing","label":"外部缺失锚"}},
            {"type":"wikiLink","attrs":{"wiki_target":"#^live","label":"有效块"}},
            {"type":"wikiLink","attrs":{"wiki_target":"HTML锚点","label":"整篇"}},
            {"type":"wikiLink","attrs":{"wiki_target":"#^removed","label":"已删块"}},
            {"type":"wikiLink","attrs":{"wiki_target":"不存在的笔记","label":"缺失目标"}}
        ]},
        {"type":"wikiEmbed","attrs":{"wiki_target":"#^missing","label":"嵌入缺失锚"}},
        {"type":"paragraph","attrs":{"block_id":"removed"},"content":[{"type":"text","text":"之后移除"}]}
    ])))).unwrap().note;
    let live_id = original.document.content[0].attrs.id.clone();
    let removed_id = original.document.content[3].attrs.id.clone();
    let mut changed = original.document.clone();
    changed.content.pop();
    let current = db
        .save_document(
            &id(),
            SaveNote {
                note_id: original.id.clone(),
                base_revision_id: original.revision_id,
                title: original.title,
                document_schema: 3,
                document: changed,
            },
        )
        .unwrap()
        .note;
    assert_eq!(
        current.document.content[1].content[0]
            .attrs
            .target_id
            .as_deref(),
        Some(current.id.as_str())
    );
    assert!(
        current.document.content[1].content[0]
            .attrs
            .target_block_id
            .is_none()
    );
    assert_eq!(
        current.document.content[1].content[6]
            .attrs
            .target_block_id
            .as_deref(),
        Some(removed_id.as_str())
    );
    let output = root.path().join("anchors.zip");
    let report = db
        .export_note_archive(&current.id, &output, ExportFormat::Html, &mut |_| Ok(()))
        .unwrap();
    let files = unpack(&output);
    let html = String::from_utf8(files[&report.document_path].clone()).unwrap();
    assert_eq!(
        html.matches("data-wiki-state=\"anchor-unresolved\"")
            .count(),
        5
    );
    assert_eq!(
        html.matches("data-wiki-state=\"anchor-missing\"").count(),
        1
    );
    assert_eq!(
        html.matches("data-wiki-state=\"target-unresolved\"")
            .count(),
        1
    );
    assert!(html.contains("缺失块 <small>（引用锚点尚未定位）</small>"));
    assert!(html.contains("已删块 <small>（引用锚点已失效）</small>"));
    assert!(html.contains(&format!("href=\"#{live_id}\">有效块</a>")));
    assert!(!html.contains(&format!("href=\"#{removed_id}\"")));
    assert_eq!(html.matches("href=\"#pkms-note-title\"").count(), 1);
    assert!(html.contains("href=\"#pkms-note-title\">整篇</a>"));
}

#[test]
fn rich_format_review_adjacent_math_refuses_merged_markdown_and_keeps_html_atoms() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db
        .create_document(
            &id(),
            create(
                "相邻公式",
                document(json!([
                    {"type":"paragraph","content":[
                        {"type":"inlineMath","attrs":{"source":"x"}},
                        {"type":"inlineMath","attrs":{"source":"y"}}
                    ]}
                ])),
            ),
        )
        .unwrap()
        .note;
    let output = root.path().join("math.zip");
    assert!(matches!(
        db.export_note_archive(&note.id, &output, ExportFormat::Markdown, &mut |_| Ok(())),
        Err(Error::InvalidInput(
            "Markdown cannot preserve adjacent unformatted inline math boundaries; choose HTML or full backup"
        ))
    ));
    assert!(!output.exists());
    let report = db
        .export_note_archive(&note.id, &output, ExportFormat::Html, &mut |_| Ok(()))
        .unwrap();
    let files = unpack(&output);
    let html = String::from_utf8(files[&report.document_path].clone()).unwrap();
    assert!(html.contains(
        "<span data-pkms-math=\"inline\">x</span><span data-pkms-math=\"inline\">y</span>"
    ));
    assert_eq!(db.note(&note.id).unwrap(), note);
}

#[test]
fn rich_format_review_table_pipe_escaping_preserves_literals_code_and_link_attributes() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let table = |content: Value| {
        document(json!([
            {"type":"table","content":[
                {"type":"tableRow","content":[{"type":"tableHeader","content":[{"type":"paragraph","content":[{"type":"text","text":"表头"}]}]}]},
                {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","content":content}]}]}
            ]}
        ]))
    };
    for (index, (kind, source)) in [
        ("comment", "a|b"),
        ("comment", r"a\\|b"),
        ("inlineMath", "|x|"),
    ]
    .into_iter()
    .enumerate()
    {
        let note = db
            .create_document(
                &id(),
                create(
                    "需表格转义的源码",
                    table(json!([{"type":kind,"attrs":{"source":source}}])),
                ),
            )
            .unwrap()
            .note;
        let output = root.path().join(format!("rejected-pipe-{index}.zip"));
        assert!(matches!(
            db.export_note_archive(&note.id, &output, ExportFormat::Markdown, &mut |_| Ok(())),
            Err(Error::InvalidInput(
                "Markdown cannot preserve literal sources requiring table escaping; choose HTML or full backup"
            ))
        ));
        assert!(!output.exists());
        let report = db
            .export_note_archive(&note.id, &output, ExportFormat::Html, &mut |_| Ok(()))
            .unwrap();
        let files = unpack(&output);
        let html = String::from_utf8(files[&report.document_path].clone()).unwrap();
        assert!(html.contains(source));
        assert_eq!(db.note(&note.id).unwrap(), note);
    }
    let note = db.create_document(&id(), create("可保真的表格", table(json!([
        {"type":"comment","attrs":{"source":r"a\|b"}},
        {"type":"text","text":" / "},
        {"type":"inlineMath","attrs":{"source":r"\|x\|"}},
        {"type":"text","text":" / "},
        {"type":"text","text":r"a\|b","marks":[{"type":"code"}]},
        {"type":"text","text":" / "},
        {"type":"text","text":"链接","marks":[{"type":"link","attrs":{"href":"https://example.invalid/a|b?left=1&right=2","title":"A | B\nC\rD"}}]},
        {"type":"footnoteReference","attrs":{"label":"a|b"}}
    ])))).unwrap().note;
    let output = root.path().join("safe-pipes.zip");
    let report = db
        .export_note_archive(&note.id, &output, ExportFormat::Markdown, &mut |_| Ok(()))
        .unwrap();
    let files = unpack(&output);
    let markdown = String::from_utf8(files[&report.document_path].clone()).unwrap();
    assert!(markdown.contains(r"%%a\|b%%"));
    assert!(markdown.contains(r"$\|x\|$"));
    assert!(markdown.contains("<code>&#97;&#92;&#124;&#98;</code>"));
    assert!(markdown.contains("href=\"https://example.invalid/a&#124;b?left=1&amp;right=2\""));
    assert!(markdown.contains("title=\"A &#124; B&#10;C&#13;D\""));
    assert!(markdown.contains("[^a&#124;b]"));
}

#[test]
fn rich_format_markdown_link_attributes_preserve_supported_values_and_absence() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_document(&id(), create("链接属性", document(json!([
        {"type":"paragraph","content":[
            {"type":"text","text":"outbound","marks":[{"type":"link","attrs":{"href":"https://example.invalid/a?left=1&right=2","title":"Title <&\"|\nline","target":"_blank","rel":"noopener noreferrer"}}]},
            {"type":"text","text":" / "},
            {"type":"text","text":"plain","marks":[{"type":"link","attrs":{"href":"https://example.invalid/plain"}}]}
        ]},
        {"type":"table","content":[
            {"type":"tableRow","content":[{"type":"tableHeader","content":[{"type":"paragraph","content":[{"type":"text","text":"链接"}]}]}]},
            {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"self","marks":[{"type":"link","attrs":{"href":"https://example.invalid/a|b","title":"","target":"_self","rel":"nofollow"}}]}]}]}]}
        ]}
    ])))).unwrap().note;
    let output = root.path().join("link-attributes.zip");
    let report = db
        .export_note_archive(&note.id, &output, ExportFormat::Markdown, &mut |_| Ok(()))
        .unwrap();
    let files = unpack(&output);
    let markdown = String::from_utf8(files[&report.document_path].clone()).unwrap();
    assert!(markdown.contains("<a href=\"https://example.invalid/a?left=1&amp;right=2\" title=\"Title &lt;&amp;&quot;&#124;&#10;line\" target=\"_blank\" rel=\"noopener noreferrer\">outbound</a>"));
    assert!(markdown.contains("<a href=\"https://example.invalid/plain\">plain</a>"));
    assert!(markdown.contains("<a href=\"https://example.invalid/a&#124;b\" title=\"\" target=\"_self\" rel=\"nofollow\">self</a>"));
    assert_eq!(db.note(&note.id).unwrap(), note);
}

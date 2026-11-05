use pkms_core::*;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn import(
    db: &mut Library,
    title: &str,
    text: &str,
    collection_id: Option<String>,
) -> SourceVersion {
    import_with_tags(db, title, text, collection_id, Vec::new())
}

fn import_with_tags(
    db: &mut Library,
    title: &str,
    text: &str,
    collection_id: Option<String>,
    tag_names: Vec<String>,
) -> SourceVersion {
    let job = db
        .accept_source_import_with_tags(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: title.into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id,
                add_version_to: None,
                base_row_version: None,
            },
            tag_names,
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
    )
    .unwrap();
    version
}

fn request(filter: KnowledgeObjectFilter, limit: u32) -> KnowledgeObjectPageRequest {
    KnowledgeObjectPageRequest {
        limit,
        cursor: None,
        query: None,
        filter: Some(filter),
    }
}

#[test]
fn knowledge_index_lists_notes_and_sources_with_category_and_preview() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let collection = db.create_collection("嵌入式开发").unwrap();
    let note = db
        .create_note("中断向量表与 RTOS 任务调度", "记录中断优先级和任务切换。")
        .unwrap();
    let source = import(
        &mut db,
        "CAN 总线故障排查手册",
        "CAN 总线错误帧、终端电阻与总线负载排查。",
        None,
    );
    db.collection_add_members(
        &id(),
        ChangeCollectionMembers {
            collection_id: collection.id.clone(),
            base_row_version: db
                .collection_get(&collection.id)
                .unwrap()
                .metadata
                .row_version,
            member_ids: vec![note.id.clone(), source.source_id.clone()],
        },
    )
    .unwrap();

    let page = db
        .knowledge_objects_page(request(KnowledgeObjectFilter::default(), 10))
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page.next_cursor.is_none());
    let note_item = page
        .items
        .iter()
        .find(|item| item.object.id == note.id)
        .unwrap();
    assert_eq!(note_item.title, "中断向量表与 RTOS 任务调度");
    assert!(note_item.preview.contains("中断优先级"));
    assert_eq!(note_item.collections[0].name, "嵌入式开发");
    assert!(note_item.note.is_some() && note_item.source.is_none());
    let source_item = page
        .items
        .iter()
        .find(|item| item.object.id == source.source_id)
        .unwrap();
    assert_eq!(source_item.title, "CAN 总线故障排查手册");
    assert!(source_item.source.is_some() && source_item.note.is_none());

    let filtered = KnowledgeObjectFilter {
        kinds: vec![ObjectKind::Source],
        collection_id: Some(collection.id.clone()),
        ..Default::default()
    };
    let page = db.knowledge_objects_page(request(filtered, 10)).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].object.id, source.source_id);
}

#[test]
fn knowledge_index_supports_uncategorized_search_and_cursor_consistency() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let collection = db.create_collection("个人研究").unwrap();
    let filed = db
        .create_note("访谈编码手册", "研究访谈的编码与证据链。")
        .unwrap();
    let loose = db
        .create_note("中断向量表笔记", "嵌入式系统中断入口。")
        .unwrap();
    db.collection_add_members(
        &id(),
        ChangeCollectionMembers {
            collection_id: collection.id.clone(),
            base_row_version: db
                .collection_get(&collection.id)
                .unwrap()
                .metadata
                .row_version,
            member_ids: vec![filed.id.clone()],
        },
    )
    .unwrap();

    let uncategorized = KnowledgeObjectFilter {
        uncategorized: true,
        ..Default::default()
    };
    let page = db
        .knowledge_objects_page(request(uncategorized, 10))
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].object.id, loose.id);

    let mut query = request(KnowledgeObjectFilter::default(), 1);
    query.query = Some("中断".into());
    let first = db.knowledge_objects_page(query.clone()).unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].object.id, loose.id);

    let mut second = request(KnowledgeObjectFilter::default(), 1);
    second.cursor = db
        .knowledge_objects_page(request(KnowledgeObjectFilter::default(), 1))
        .unwrap()
        .next_cursor;
    assert!(second.cursor.is_some());
    let page = db.knowledge_objects_page(second).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_ne!(page.items[0].object.id, loose.id);
}

#[test]
fn knowledge_index_rejects_mixed_uncategorized_and_collection_filters() {
    let root = tempdir().unwrap();
    let db = Library::open(root.path()).unwrap();
    let filter = KnowledgeObjectFilter {
        uncategorized: true,
        collection_id: Some(id()),
        ..Default::default()
    };
    assert!(db.knowledge_objects_page(request(filter, 10)).is_err());
}
#[test]
fn categorized_import_is_processed_but_unclassified_import_stays_in_inbox() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let collection = db.create_collection("嵌入式开发").unwrap();
    let filed = import(
        &mut db,
        "SPI 时序与中断优先级",
        "SPI 时钟极性、中断优先级与 RTOS 任务调度。",
        Some(collection.id),
    );
    let loose = import(
        &mut db,
        "CAN 总线故障排查",
        "终端电阻、错误帧与总线负载。",
        None,
    );
    assert_eq!(
        db.object_metadata(&filed.source_id).unwrap().inbox_state,
        Some(InboxState::Processed)
    );
    assert_eq!(
        db.object_metadata(&loose.source_id).unwrap().inbox_state,
        Some(InboxState::Pending)
    );
}

#[test]
fn tag_only_import_creates_tags_and_finishes_organization_atomically() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let source = import_with_tags(
        &mut db,
        "CAN 总线故障复盘",
        "确认 120Ω 终端电阻、错误帧与采样丢帧之间的因果关系。",
        None,
        vec!["故障排查".into(), "CAN 总线".into(), "故障排查".into()],
    );
    let metadata = db.object_metadata(&source.source_id).unwrap();
    assert_eq!(metadata.inbox_state, Some(InboxState::Processed));
    let names: Vec<_> = metadata
        .tags
        .iter()
        .map(|tag| tag.display_name.as_str())
        .collect();
    assert_eq!(names, vec!["CAN 总线", "故障排查"]);
    let filtered = KnowledgeObjectFilter {
        tag_ids: metadata.tags.iter().map(|tag| tag.id.clone()).collect(),
        ..Default::default()
    };
    let page = db.knowledge_objects_page(request(filtered, 10)).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].object.id, source.source_id);
}

#[test]
fn knowledge_index_filters_grouped_search_ids_without_losing_source_context() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let rtos = db
        .create_note(
            "RTOS 任务调度与中断优先级",
            "记录中断向量、任务优先级反转和临界区长度。",
        )
        .unwrap();
    let interview = db
        .create_note(
            "访谈编码：研究问题如何收束",
            "把访谈片段编码为研究问题、证据和反例。",
        )
        .unwrap();
    let spi = import(
        &mut db,
        "SPI 时序与 CAN 总线故障排查",
        "SPI 时钟极性、CAN 终端电阻与错误帧排查记录。",
        None,
    );

    let grouped = KnowledgeObjectFilter {
        object_ids: vec![rtos.id.clone(), spi.source_id.clone()],
        ..Default::default()
    };
    let page = db.knowledge_objects_page(request(grouped, 10)).unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page.items.iter().any(|item| item.title.contains("RTOS")));
    assert!(page.items.iter().any(|item| item.title.contains("SPI")));
    assert!(!page.items.iter().any(|item| item.object.id == interview.id));

    let invalid = KnowledgeObjectFilter {
        object_ids: vec!["not-a-object-id".into()],
        ..Default::default()
    };
    assert!(db.knowledge_objects_page(request(invalid, 10)).is_err());
}

#[test]
fn knowledge_index_includes_source_excerpts_as_readable_objects() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let source = import(
        &mut db,
        "设计数据密集型应用-第128页",
        "第 128 页讨论复制与分区：系统必须在延迟、可用性和一致性之间做取舍。",
        None,
    );
    let collection = db.create_collection("阅读与摘录").unwrap();
    db.collection_add_members(
        &id(),
        ChangeCollectionMembers {
            collection_id: collection.id.clone(),
            base_row_version: db
                .collection_get(&collection.id)
                .unwrap()
                .metadata
                .row_version,
            member_ids: vec![source.source_id.clone()],
        },
    )
    .unwrap();
    let annotation = annotate(
        &mut db,
        &source,
        "复制延迟是设计必须暴露给使用者的状态，不是偶发异常。",
    );
    let select_excerpts = KnowledgeObjectFilter {
        kinds: vec![ObjectKind::Annotation],
        ..Default::default()
    };
    let page = db
        .knowledge_objects_page(request(select_excerpts.clone(), 10))
        .unwrap();
    assert_eq!(page.items.len(), 1);
    let item = &page.items[0];
    let inbox = KnowledgeObjectFilter {
        kinds: vec![ObjectKind::Annotation],
        inbox_state: Some(InboxState::Pending),
        ..Default::default()
    };
    let inbox_page = db
        .knowledge_objects_page(request(inbox.clone(), 10))
        .unwrap();
    assert_eq!(inbox_page.items.len(), 1);
    assert_eq!(inbox_page.items[0].object.id, annotation.id);
    assert_eq!(
        inbox_page.items[0].object.inbox_state,
        Some(InboxState::Pending)
    );
    let snoozed = db
        .set_inbox_state(
            &id(),
            SetInboxState {
                object_id: annotation.id.clone(),
                base_row_version: inbox_page.items[0].object.row_version,
                target: InboxState::Snoozed,
            },
        )
        .unwrap();
    assert_eq!(snoozed.objects[0].inbox_state, Some(InboxState::Snoozed));
    let empty_inbox = db.knowledge_objects_page(request(inbox, 10)).unwrap();
    assert!(empty_inbox.items.is_empty());
    assert_eq!(item.object.id, annotation.id);
    assert!(item.annotation.is_some());
    assert!(item.title.contains("设计数据密集型应用-第128页"));
    assert!(item.preview.contains("复制延迟是设计必须暴露"));
    assert_eq!(item.collections[0].name, "阅读与摘录");

    let categorized = KnowledgeObjectFilter {
        kinds: vec![ObjectKind::Annotation],
        collection_id: Some(collection.id),
        ..Default::default()
    };
    let categorized_page = db.knowledge_objects_page(request(categorized, 10)).unwrap();
    assert_eq!(categorized_page.items.len(), 1);
    assert_eq!(categorized_page.items[0].object.id, annotation.id);

    let mut quote_query = request(select_excerpts.clone(), 10);
    quote_query.query = Some("复制延迟".into());
    let by_quote = db.knowledge_objects_page(quote_query).unwrap();
    assert_eq!(by_quote.items.len(), 1);
    assert_eq!(by_quote.items[0].object.id, annotation.id);
    let mut comment_query = request(select_excerpts, 10);
    comment_query.query = Some("复制延迟是设计必须暴露".into());
    let by_comment = db.knowledge_objects_page(comment_query).unwrap();
    assert_eq!(by_comment.items.len(), 1);
    assert_eq!(by_comment.items[0].object.id, annotation.id);
}

fn annotate(db: &mut Library, source: &SourceVersion, comment: &str) -> Annotation {
    let block = db
        .source_text_page(&source.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    db.create_annotation(
        &id(),
        CreateAnnotation {
            locator: SourceLocator::Text {
                source_version_id: source.id.clone(),
                block_id: block.id,
                start_codepoint: 0,
                end_codepoint: block.text.chars().count() as u32,
                quote: block.text,
                prefix: String::new(),
                suffix: String::new(),
            },
            comment: comment.into(),
        },
    )
    .unwrap()
    .annotation
    .unwrap()
}

#[test]
fn knowledge_index_exact_phrase_reaches_note_and_source_body() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db
        .create_note(
            "访谈研究记录",
            "第一轮开放式编码得到 18 个概念。反例必须保留，否则结论只是在重复自己的假设。",
        )
        .unwrap();
    let source = import(
        &mut db,
        "控制板通信故障复盘",
        "CAN 总线错误帧增加时，先检查 120Ω 终端电阻，再看总线负载。",
        None,
    );

    let mut body_query = request(KnowledgeObjectFilter::default(), 10);
    body_query.query = Some("反例必须保留".into());
    let body = db.knowledge_objects_page(body_query).unwrap();
    assert_eq!(body.items.len(), 1);
    assert_eq!(body.items[0].object.id, note.id);
    let body_hit = body.items[0].hit.as_ref().expect("note body hit");
    assert!(body_hit.snippet.contains("反例必须保留"));
    assert!(body_hit.block_id.is_some());
    assert!(!body.truncated);

    let mut source_query = request(KnowledgeObjectFilter::default(), 10);
    source_query.query = Some("can 总线错误帧".into());
    let source_result = db.knowledge_objects_page(source_query).unwrap();
    assert_eq!(source_result.items.len(), 1);
    assert_eq!(source_result.items[0].object.id, source.source_id);
    let source_hit = source_result.items[0]
        .hit
        .as_ref()
        .expect("source body hit");
    assert!(source_hit.snippet.contains("CAN 总线错误帧"));
    assert!(source_hit.block_id.is_some());
}

#[test]
fn knowledge_index_reports_when_the_requested_page_is_truncated() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    db.create_note("CAN 总线复盘 A", "错误帧与终端电阻。")
        .unwrap();
    db.create_note("CAN 总线复盘 B", "采样点与总线负载。")
        .unwrap();
    let mut query = request(KnowledgeObjectFilter::default(), 1);
    query.query = Some("CAN 总线".into());
    let page = db.knowledge_objects_page(query).unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.truncated);
    assert!(page.next_cursor.is_some());
}

#[test]
fn note_created_in_a_category_is_already_organized() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let collection = db.create_collection("嵌入式开发").unwrap();
    let note = db
        .create_document(
            &id(),
            CreateNote {
                title: "中断向量表与 RTOS 任务调度".into(),
                document_schema: DOCUMENT_SCHEMA,
                document: NoteDocument::from_text(
                    id(),
                    "中断优先级分组会影响 SPI 采样与 CAN 接收。",
                ),
                collection_id: Some(collection.id.clone()),
                base_collection_row_version: Some(
                    db.collection_get(&collection.id)
                        .unwrap()
                        .metadata
                        .row_version,
                ),
            },
        )
        .unwrap()
        .note;
    assert_eq!(
        db.object_metadata(&note.id).unwrap().inbox_state,
        Some(InboxState::Processed)
    );
    let page = db
        .knowledge_objects_page(request(
            KnowledgeObjectFilter {
                collection_id: Some(collection.id),
                inbox_state: Some(InboxState::Processed),
                ..Default::default()
            },
            10,
        ))
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].object.id, note.id);
}
#[test]
fn frontmatter_relation_counts_are_bidirectional_in_knowledge_index() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let target = db
        .create_note(
            "CAN 错误帧与终端电阻复核",
            "检查 120Ω 终端电阻和错误帧计数。",
        )
        .unwrap();
    let document = serde_json::from_value(json!({
        "type": "doc",
        "content": [
            {
                "type": "frontmatter",
                "attrs": {
                    "id": id(),
                    "source": format!("type: note\nrelated:\n  - kind: note\n    id: {}\n    title: CAN 错误帧与终端电阻复核\n", target.id)
                }
            },
            {
                "type": "paragraph",
                "attrs": { "id": id() },
                "content": [{ "type": "text", "text": "SPI 采样与 CAN 接收共享同一条故障排查路径。" }]
            }
        ]
    }))
    .unwrap();
    let source = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "SPI 时序与 CAN 总线故障排查".into(),
                document_schema: 3,
                document,
            },
        )
        .unwrap()
        .note;

    let page = db
        .knowledge_objects_page(request(KnowledgeObjectFilter::default(), 10))
        .unwrap();
    let source_item = page
        .items
        .iter()
        .find(|item| item.object.id == source.id)
        .expect("source item");
    let target_item = page
        .items
        .iter()
        .find(|item| item.object.id == target.id)
        .expect("target item");
    assert_eq!(source_item.related_count, 1);
    assert_eq!(target_item.related_count, 1);
}

#[test]
fn knowledge_index_search_uses_full_excerpt_and_comment_beyond_preview() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let quote_prefix = "复制延迟需要把主库确认、从库应用和读一致性窗口分开记录。".repeat(12);
    let quote_marker = "末尾摘录标记：跨分区写入顺序必须可重建";
    let source_text = format!("{quote_prefix}{quote_marker}");
    let comment_prefix = "研究批注先区分观察、解释与反例，再检查证据链是否闭合。".repeat(12);
    let comment_marker = "末尾批注标记：反例必须保留";
    let comment = format!("{comment_prefix}{comment_marker}");
    let source = import(&mut db, "设计数据密集型应用-第128页", &source_text, None);
    let annotation = annotate(&mut db, &source, &comment);
    let excerpt_filter = KnowledgeObjectFilter {
        kinds: vec![ObjectKind::Annotation],
        ..Default::default()
    };

    let page = db
        .knowledge_objects_page(request(excerpt_filter.clone(), 10))
        .unwrap();
    let item = page.items.first().expect("annotation item");
    assert_eq!(item.object.id, annotation.id);
    let summary = item.annotation.as_ref().expect("annotation summary");
    assert!(!summary.quote_preview.contains(quote_marker));
    assert!(!summary.comment_preview.contains(comment_marker));

    let mut quote_query = request(excerpt_filter.clone(), 10);
    quote_query.query = Some(quote_marker.into());
    let by_quote = db.knowledge_objects_page(quote_query).unwrap();
    assert_eq!(by_quote.items.len(), 1);
    assert!(
        by_quote.items[0]
            .hit
            .as_ref()
            .expect("quote hit")
            .snippet
            .contains(quote_marker)
    );

    let mut comment_query = request(excerpt_filter, 10);
    comment_query.query = Some(comment_marker.into());
    let by_comment = db.knowledge_objects_page(comment_query).unwrap();
    assert_eq!(by_comment.items.len(), 1);
    assert!(
        by_comment.items[0]
            .hit
            .as_ref()
            .expect("comment hit")
            .snippet
            .contains(comment_marker)
    );
}

#[test]
fn numbered_browse_counts_and_direct_pages_match_the_visible_cursor_rows() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let category = db.create_collection("直接分页分类").unwrap();
    let mut members = Vec::new();
    for index in 0..101 {
        members.push(
            db.create_note(&format!("分页条目 {index:03}"), "真实分页正文")
                .unwrap()
                .id,
        );
    }
    for batch in members.chunks(100) {
        db.collection_add_members(
            &id(),
            ChangeCollectionMembers {
                collection_id: category.id.clone(),
                base_row_version: db
                    .collection_get(&category.id)
                    .unwrap()
                    .metadata
                    .row_version,
                member_ids: batch.to_vec(),
            },
        )
        .unwrap();
    }
    db.create_note("集合外条目", "不应计入选中集合").unwrap();
    let filter = KnowledgeObjectFilter {
        collection_id: Some(category.id),
        ..Default::default()
    };
    let browse = KnowledgeBrowseRequest {
        limit: 10,
        page: 11,
        filter: Some(filter.clone()),
        ..Default::default()
    };
    let last = db.knowledge_browse(browse.clone()).unwrap();
    assert_eq!(
        (last.total, last.page, last.page_size, last.items.len()),
        (101, 11, 10, 1)
    );
    assert!(last.next_cursor.is_none());
    assert!(!last.truncated);
    assert_eq!(
        last.type_counts
            .iter()
            .map(|entry| entry.count)
            .sum::<u32>(),
        101
    );
    let mut cursor_request = request(filter, 100);
    let first = db.knowledge_objects_page(cursor_request.clone()).unwrap();
    cursor_request.cursor = first.next_cursor;
    let cursor_last = db.knowledge_objects_page(cursor_request).unwrap();
    assert_eq!(last.items[0].object.id, cursor_last.items[0].object.id);
    let clamped = db
        .knowledge_browse(KnowledgeBrowseRequest {
            page: u32::MAX,
            ..browse.clone()
        })
        .unwrap();
    assert_eq!(clamped.page, 11);
    assert!(
        db.knowledge_browse(KnowledgeBrowseRequest { page: 0, ..browse })
            .is_err()
    );
}

#[test]
fn numbered_browse_shares_annotation_membership_and_validates_extra_filters() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let category = db.create_collection("资料集合").unwrap();
    let source = import(
        &mut db,
        "中文资料",
        "用于摘要和摘录的完整证据正文",
        Some(category.id.clone()),
    );
    let annotation = annotate(&mut db, &source, "共享来源分类");
    db.create_note("集合外中文笔记", "完整证据正文").unwrap();
    let browse = KnowledgeBrowseRequest {
        limit: 10,
        page: 1,
        filter: Some(KnowledgeObjectFilter {
            collection_id: Some(category.id),
            ..Default::default()
        }),
        source_formats: vec![SourceFormat::Text],
        title_query: Some("中文".into()),
        ..Default::default()
    };
    let page = db.knowledge_browse(browse.clone()).unwrap();
    assert_eq!(page.total, 2);
    assert!(
        page.items
            .iter()
            .any(|entry| entry.object.id == annotation.id)
    );
    assert_eq!(page.type_counts.len(), 2);
    let empty = db
        .knowledge_browse(KnowledgeBrowseRequest {
            source_formats: vec![SourceFormat::Pdf],
            ..browse.clone()
        })
        .unwrap();
    assert_eq!((empty.total, empty.page, empty.items.len()), (0, 1, 0));
    assert!(
        db.knowledge_browse(KnowledgeBrowseRequest {
            updated_from: Some("not-a-date".into()),
            ..browse.clone()
        })
        .is_err()
    );
    assert!(
        db.knowledge_browse(KnowledgeBrowseRequest {
            updated_from: Some("2026-11-05T00:00:00Z".into()),
            updated_to: Some("2026-11-04T00:00:00Z".into()),
            ..browse
        })
        .is_err()
    );
}

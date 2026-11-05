use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn query() -> SearchQuery {
    SearchQuery {
        query: "中文知识".into(),
        mode: SearchMode::Lexical,
        scope: SearchScope::default(),
        limit: 100,
        minimum_cosine: None,
    }
}
fn source(db: &mut Library, text: &str) -> SourceVersion {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "资料.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let bytes = text.as_bytes();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, bytes).unwrap(),
    )
    .unwrap();
    version
}
#[test]
fn chinese_keyword_search_without_model_keeps_real_note_and_source_targets() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let note = db
        .create_note("中文知识笔记", "中文知识来自真实文本😀")
        .unwrap();
    let source = source(&mut db, "中文知识的真实来源");
    let result = db.keyword_search(&query()).unwrap();
    assert!(result.lexical_available);
    assert!(!result.semantic_available);
    assert_eq!(result.items.len(), 2);
    for (rank, hit) in result.items.iter().enumerate() {
        assert_eq!(hit.lexical_rank, Some(rank as u32 + 1));
        assert!(hit.cosine_similarity.is_none());
        let input = db.index_input(&hit.target.object_id).unwrap();
        let block = input
            .blocks
            .iter()
            .find(|b| b.target.block_id == hit.target.block_id)
            .unwrap();
        assert_eq!(hit.text, block.text);
        assert_eq!(hit.target.content_revision, input.content_revision);
    }
    assert!(
        result
            .items
            .iter()
            .any(|h| h.target.object_id == note.id && h.target.parse_id.is_none())
    );
    assert!(
        result
            .items
            .iter()
            .any(|h| h.target.object_id == source.source_id && h.target.parse_id.is_some())
    );
    let mut q = query();
    q.scope.kinds = vec![EvidenceKind::Source];
    assert_eq!(db.keyword_search(&q).unwrap().items.len(), 1);
    q.scope.source_format = Some(SourceFormat::Pdf);
    assert!(db.keyword_search(&q).unwrap().items.is_empty());
    q.scope.source_format = Some(SourceFormat::Text);
    q.scope.reading_status = Some(ReadingStatus::Unread);
    assert_eq!(db.keyword_search(&q).unwrap().items.len(), 1);
    q.scope.reading_status = Some(ReadingStatus::Finished);
    assert!(db.keyword_search(&q).unwrap().items.is_empty());
}
#[test]
fn scope_filters_before_limit_and_combines_tags_collection_inbox_and_archive() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    for _ in 0..105 {
        db.create_note("中文知识", "中文知识").unwrap();
    }
    let wanted = db.create_note("中文知识", "中文知识指定结果").unwrap();
    let collection = db.create_collection("检索范围").unwrap();
    db.add_to_collection(&collection.id, &wanted.id).unwrap();
    let mut tag_ids = vec![];
    for name in ["甲", "乙"] {
        let metadata = db.object_metadata(&wanted.id).unwrap();
        let result = db
            .assign_tag(
                &id(),
                AssignTag {
                    targets: vec![MetadataTarget {
                        object_id: wanted.id.clone(),
                        base_row_version: metadata.row_version,
                    }],
                    tag: TagSelector::Name { name: name.into() },
                },
            )
            .unwrap();
        tag_ids.push(result.tag.unwrap().id);
    }
    let mut q = query();
    q.limit = 1;
    q.scope.object_ids = vec![wanted.id.clone()];
    q.scope.tag_ids = tag_ids;
    q.scope.collection_id = Some(collection.id);
    q.scope.inbox_state = Some(InboxState::Pending);
    q.scope.archived = Some(false);
    assert_eq!(
        db.keyword_search(&q).unwrap().items[0].target.object_id,
        wanted.id
    );
    q.scope.object_ids.clear();
    assert_eq!(
        db.keyword_search(&q).unwrap().items[0].target.object_id,
        wanted.id
    );
    q.scope.tag_ids.push(id());
    assert!(db.keyword_search(&q).unwrap().items.is_empty());
    q.scope.tag_ids.pop();
    q.scope.inbox_state = Some(InboxState::Processed);
    assert!(db.keyword_search(&q).unwrap().items.is_empty());
    q.scope.inbox_state = None;
    q.scope.archived = Some(true);
    assert!(db.keyword_search(&q).unwrap().items.is_empty());
}
#[test]
fn index_id_pages_are_bounded_and_query_inputs_are_validated() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    for _ in 0..3 {
        db.create_note("中文知识", "内容").unwrap();
    }
    let first = db.index_object_ids(None, 2).unwrap();
    let next = db
        .index_object_ids(first.last().map(String::as_str), 2)
        .unwrap();
    assert_eq!(next.len(), 1);
    assert!(first.iter().all(|i| i < &next[0]));
    let mut q = query();
    q.limit = 0;
    assert!(db.keyword_search(&q).is_err());
    q.limit = 1;
    q.scope.object_ids = vec!["wrong".into()];
    assert!(db.keyword_search(&q).is_err());
    q.scope.object_ids = vec![id(); 101];
    assert!(db.keyword_search(&q).is_err());
}

#[test]
fn source_scope_is_applied_before_the_source_candidate_limit() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let _other = source(&mut db, "中文知识");
    let wanted = source(
        &mut db,
        "中文知识的真实来源包含额外的背景说明用于降低匹配排名",
    );
    let mut q = query();
    q.limit = 1;
    q.scope.object_ids = vec![wanted.source_id.clone()];
    let results = db.keyword_search(&q).unwrap();
    assert_eq!(results.items.len(), 1);
    assert_eq!(results.items[0].target.object_id, wanted.source_id);
    db.set_reading_state(
        &id(),
        SetReadingState {
            pdf_view: None,
            source_id: wanted.source_id.clone(),
            source_version_id: wanted.id,
            locator: None,
            status: ReadingStatus::Finished,
            base_row_version: 0,
        },
    )
    .unwrap();
    q.scope.reading_status = Some(ReadingStatus::Finished);
    assert_eq!(db.keyword_search(&q).unwrap().items.len(), 1);
    q.scope.reading_status = Some(ReadingStatus::Unread);
    assert!(db.keyword_search(&q).unwrap().items.is_empty());
}

#[test]
fn ranked_questions_recall_notes_without_requiring_question_words() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let chinese = db
        .create_note("银杉实验", "银杉实验每周五备份，保留三个月。")
        .unwrap();
    let english = db
        .create_note(
            "Kestrel",
            "Kestrel cache capacity: 256 MiB. Reset with cache.flush().",
        )
        .unwrap();
    db.create_note("日常记录", "今天的午餐很好，我们在楼下见面。")
        .unwrap();
    db.create_note("Other service", "The other service is available at noon.")
        .unwrap();
    let mut q = query();
    q.limit = 1;
    for (question, expected) in [
        ("银杉实验什么时候备份，保留多久？", &chinese.id),
        ("What is the capacity of Kestrel cache?", &english.id),
        ("How do I call cache.flush() for Kestrel?", &english.id),
    ] {
        q.query = question.into();
        let result = db.keyword_search(&q).unwrap();
        assert_eq!(&result.items[0].target.object_id, expected, "{question}");
    }
}

#[test]
fn ranked_keywords_favor_joint_matches_and_quote_fts_operators() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("one", "saffron only").unwrap();
    db.create_note("two", "quartz only").unwrap();
    let both = db.create_note("three", "saffron quartz").unwrap();
    let mut q = query();
    q.query = "saffron quartz".into();
    q.limit = 1;
    assert_eq!(
        db.keyword_search(&q).unwrap().items[0].target.object_id,
        both.id
    );
    q.query = "\"saffron\" OR NEAR(quartz) NOT *".into();
    assert_eq!(
        db.keyword_search(&q).unwrap().items[0].target.object_id,
        both.id
    );
    q.query = "absentuniqueterm".into();
    assert!(db.keyword_search(&q).unwrap().items.is_empty());
    q.query = "!? \" *".into();
    assert!(db.keyword_search(&q).unwrap().items.is_empty());
}

#[test]
fn exact_source_outranks_a_note_matching_only_a_question_word() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("unrelated", "the service is open").unwrap();
    let wanted = source(&mut db, "Kestrel cache capacity: 256 MiB.");
    let mut q = query();
    q.query = "What is the capacity of Kestrel cache?".into();
    q.limit = 1;
    assert_eq!(
        db.keyword_search(&q).unwrap().items[0].target.object_id,
        wanted.source_id
    );
}

#[test]
fn long_evidence_selects_the_relevant_window_with_original_unicode_offsets() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("unrelated", "the service is open").unwrap();
    // U+0130 lowercases to two codepoints; these must not shift original locators.
    let text = format!(
        "the {} Kestrel cache capacity: 256 MiB.",
        "İ😀padding ".repeat(400)
    );
    let wanted = source(&mut db, &text);
    let mut q = query();
    q.query = "What is the capacity of Kestrel cache?".into();
    q.limit = 1;
    let result = db.keyword_search(&q).unwrap();
    let hit = &result.items[0];
    assert_eq!(hit.target.object_id, wanted.source_id);
    assert!(hit.text.contains("Kestrel cache capacity: 256 MiB."));
    assert!(hit.target.start_codepoint > 0);
    assert_eq!(hit.text.chars().count(), 2048);
    assert_eq!(
        hit.text,
        text.chars()
            .skip(hit.target.start_codepoint as usize)
            .take(2048)
            .collect::<String>()
    );
}

#[test]
fn note_evidence_selects_real_terms_instead_of_substrings_in_another_block() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let note = db.create_note("reference", "initial").unwrap();
    let wanted_block = id();
    let document = serde_json::from_value(serde_json::json!({
        "type": "doc", "content": [
            {"type":"paragraph", "attrs":{"id":wanted_block},
             "content":[{"type":"text", "text":"alpha beta gamma 42"}]},
            {"type":"paragraph", "attrs":{"id":id()},
             "content":[{"type":"text", "text":"alphabet betamax gammascope"}]}
        ]
    }))
    .unwrap();
    db.save_document(
        &id(),
        SaveNote {
            note_id: note.id.clone(),
            base_revision_id: note.revision_id,
            title: "reference".into(),
            document_schema: 2,
            document,
        },
    )
    .unwrap();
    let mut q = query();
    q.query = "alpha beta gamma".into();
    let result = db.keyword_search(&q).unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].target.block_id, wanted_block);
    assert_eq!(result.items[0].text, "alpha beta gamma 42");
}

#[test]
fn search_preflight_and_result_paths_share_request_bounds() {
    let dir = tempdir().unwrap();
    let db = Library::open(dir.path()).unwrap();
    for mode in [
        SearchMode::Lexical,
        SearchMode::Semantic,
        SearchMode::Hybrid,
    ] {
        let mut valid = query();
        valid.mode = mode;
        valid.query = "😀".repeat(1024); // The limit is bytes, not codepoints.
        valid.scope.object_ids = vec![id(); 100];
        valid.scope.tag_ids = vec![id(); 100];
        valid.scope.collection_id = Some(id());
        valid.minimum_cosine = Some(-1.0);
        assert!(valid.validate().is_ok());
        assert!(db.search_with_vectors(&valid, vec![]).is_ok());
        let mut cases = Vec::new();
        for limit in [0, 101] {
            let mut q = valid.clone();
            q.limit = limit;
            cases.push(q);
        }
        let mut q = valid.clone();
        q.query.push('a');
        cases.push(q);
        let mut q = valid.clone();
        q.scope.object_ids.push(id());
        cases.push(q);
        let mut q = valid.clone();
        q.scope.tag_ids.push(id());
        cases.push(q);
        let mut q = valid.clone();
        q.scope.kinds = vec![EvidenceKind::Note; 3];
        cases.push(q);
        let mut q = valid.clone();
        q.scope.object_ids[0] = "bad-object".into();
        cases.push(q);
        let mut q = valid.clone();
        q.scope.tag_ids[0] = "bad-tag".into();
        cases.push(q);
        let mut q = valid.clone();
        q.scope.collection_id = Some("bad-collection".into());
        cases.push(q);
        if mode != SearchMode::Lexical {
            for minimum in [f64::NAN, f64::INFINITY, -1.001, 1.001] {
                let mut q = valid.clone();
                q.minimum_cosine = Some(minimum);
                cases.push(q);
            }
        }
        for invalid in cases {
            assert!(matches!(invalid.validate(), Err(Error::InvalidInput(_))));
            assert!(matches!(
                db.keyword_search(&invalid),
                Err(Error::InvalidInput(_))
            ));
            assert!(matches!(
                db.search_with_vectors(&invalid, vec![]),
                Err(Error::InvalidInput(_))
            ));
        }
    }
}

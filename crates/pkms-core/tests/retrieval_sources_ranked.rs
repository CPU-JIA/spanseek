use pkms_core::*;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn capture(
    db: &mut Library,
    name: &str,
    text: &str,
    source: Option<&SourceSummary>,
) -> (SourceVersion, SourceJobLease) {
    let accepted = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: name.into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: source.map(|source| source.id.clone()),
                base_row_version: source.map(|source| source.row_version),
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    (version, lease)
}

fn import(
    db: &mut Library,
    name: &str,
    text: &str,
    source: Option<&SourceSummary>,
) -> SourceVersion {
    let (version, lease) = capture(db, name, text, source);
    db.publish_source_parse(
        &lease.job.id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
    )
    .unwrap();
    version
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        query: text.into(),
        mode: SearchMode::Lexical,
        scope: SearchScope {
            kinds: vec![EvidenceKind::Source],
            ..Default::default()
        },
        limit: 100,
        minimum_cosine: None,
    }
}

#[test]
fn source_question_ranks_title_and_body_without_replacing_matching_block_by_first_block() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let wanted = import(&mut db, "nebula", "introduction\ncobalt", None);
    import(&mut db, "other", "cobalt cobalt cobalt", None);
    let title_only = import(&mut db, "nebula archive", "firstplain\nsecondplain", None);
    let result = db
        .keyword_search(&query("nebula cobalt extraneousquestion"))
        .unwrap();
    assert_eq!(result.items[0].target.object_id, wanted.source_id);
    assert_eq!(result.items[0].text, "cobalt");
    let second_block = db.source_text_page(&wanted.id, None, 1, 1).unwrap();
    assert_eq!(result.items[0].target.block_id, second_block.blocks[0].id);
    let title_hits = result
        .items
        .iter()
        .filter(|item| item.target.object_id == title_only.source_id)
        .collect::<Vec<_>>();
    assert_eq!(title_hits.len(), 1);
    assert_eq!(title_hits[0].text, "firstplain");
    assert!(
        db.search_sources("nebula cobalt extraneousquestion", 100)
            .unwrap()
            .is_empty(),
        "source browser must retain strict AND semantics"
    );
}

#[test]
fn source_ranked_scope_lifecycle_history_and_renamed_title_are_filtered_before_limit() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    for n in 0..4 {
        import(
            &mut db,
            "quasar",
            &format!("quasar quasar competitor {n}"),
            None,
        );
    }
    let wanted = import(&mut db, "formername", "quasar wanted", None);
    let mut request = query("quasar extraquestion");
    request.limit = 1;
    request.scope.object_ids = vec![wanted.source_id.clone()];
    assert_eq!(
        db.keyword_search(&request).unwrap().items[0]
            .target
            .object_id,
        wanted.source_id
    );
    let source = db.source(&wanted.source_id).unwrap();
    db.update_source_metadata(
        &id(),
        UpdateSourceMetadata {
            source_id: source.id,
            base_row_version: source.row_version,
            display_title: "newname".into(),
        },
    )
    .unwrap();
    request.query = "formername".into();
    assert!(
        db.keyword_search(&request).unwrap().items.is_empty(),
        "legacy per-block title_tokens must not participate"
    );
    let source = db.source(&wanted.source_id).unwrap();
    let next = import(&mut db, "replacement", "current zircon", Some(&source));
    request.query = "quasar".into();
    assert!(db.keyword_search(&request).unwrap().items.is_empty());
    request.query = "zircon".into();
    assert_eq!(
        db.keyword_search(&request).unwrap().items[0]
            .target
            .content_revision,
        next.id
    );
    let source = db.source(&wanted.source_id).unwrap();
    db.set_source_trashed(
        &id(),
        SetObjectFlag {
            object_id: source.id,
            base_row_version: source.row_version,
            value: true,
        },
    )
    .unwrap();
    assert!(db.keyword_search(&request).unwrap().items.is_empty());
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE objects SET search_enabled=0 WHERE kind='source'",
        [],
    )
    .unwrap();
    assert!(
        db.keyword_search(&query("quasar"))
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn source_question_recalls_chinese_evidence_but_never_exposes_staged_parse() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let source = import(&mut db, "物理资料", "量子纠缠描述粒子之间的关联。", None);
    let results = db
        .keyword_search(&query("请解释量子纠缠的含义以及应用？"))
        .unwrap();
    assert!(
        results
            .items
            .iter()
            .any(|item| item.target.object_id == source.source_id)
    );
    let text = "stagedword\n".repeat(350);
    let (version, lease) = capture(&mut db, "stagedtitle", &text, None);
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER cancel_ranked_parse AFTER INSERT ON source_blocks
        WHEN NEW.position=100 BEGIN UPDATE source_jobs SET cancel_requested=1; END;",
    )
    .unwrap();
    assert!(
        db.publish_source_parse(
            &lease.job.id,
            &lease.lease_token,
            parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap()
        )
        .is_err()
    );
    assert!(
        db.keyword_search(&query("stagedword stagedtitle"))
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn source_distinct_term_coverage_outranks_a_weak_title_and_body_double_vote() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let weak = import(&mut db, "the manual", "the", None);
    let strong = import(&mut db, "reference", "kestrel cache capacity 256 mib", None);
    let mut request = query("What is the capacity of Kestrel cache?");
    request.limit = 1;
    let results = db.keyword_search(&request).unwrap();
    assert_eq!(results.items.len(), 1);
    assert_eq!(results.items[0].target.object_id, strong.source_id);
    assert_ne!(results.items[0].target.object_id, weak.source_id);
    assert_eq!(results.items[0].text, "kestrel cache capacity 256 mib");

    // Repeating a term in the query or both source fields gives no extra coverage.
    request.query = "the the the capacity kestrel cache".into();
    assert_eq!(
        db.keyword_search(&request).unwrap().items[0]
            .target
            .object_id,
        strong.source_id
    );
}

#[test]
fn source_coverage_combines_title_with_one_block_without_substring_or_cross_block_matches() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let strong = import(&mut db, "alpha", "beta gamma", None);
    import(
        &mut db,
        "alpha alphabet",
        "betamax gammascope\nbeta\ngamma",
        None,
    );
    let mut request = query("alpha beta gamma");
    request.limit = 1;
    let results = db.keyword_search(&request).unwrap();
    assert_eq!(results.items[0].target.object_id, strong.source_id);
    assert_eq!(results.items[0].text, "beta gamma");
}

#[test]
fn a_boilerplate_term_leaves_the_source_body_match_without_losing_selective_hits() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    // 36 sources repeat only the template word; 4 also carry the word the
    // question turns on. The repeating word is in 40 sources, which passes the
    // boilerplate floor (32) and beats the rarest term (4) by more than the
    // ratio (8), so it cannot separate sources for this question.
    for index in 0..36 {
        import(
            &mut db,
            &format!("filler-{index}"),
            &format!("保留 模板段落 {index}"),
            None,
        );
    }
    let mut selective = Vec::new();
    for index in 0..4 {
        selective.push(import(
            &mut db,
            &format!("selective-{index}"),
            &format!("保留 罕见词 {index}"),
            None,
        ));
    }
    let results = db.keyword_search(&query("保留 罕见词")).unwrap();
    assert!(
        results
            .items
            .iter()
            .all(|item| item.text.contains("罕见词")),
        "the template word must stop pulling every block into the ranked set: {:?}",
        results
            .items
            .iter()
            .map(|item| &item.text)
            .collect::<Vec<_>>()
    );
    for version in &selective {
        assert!(
            results
                .items
                .iter()
                .any(|item| item.target.object_id == version.source_id),
            "a source that carries the selective word must stay reachable"
        );
    }
}

#[test]
fn the_rarest_term_alone_is_never_pruned_away() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    for index in 0..40 {
        import(
            &mut db,
            &format!("filler-{index}"),
            &format!("保留 模板段落 {index}"),
            None,
        );
    }
    let results = db.keyword_search(&query("保留")).unwrap();
    assert_eq!(
        results.items.len(),
        40,
        "a one-term query keeps every match: the rarest term always survives"
    );
}

#[test]
fn a_term_below_the_boilerplate_floor_still_matches_every_document() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    // 26 repeating sources keep the term under the floor of 32, so nothing is
    // pruned and every match stays in the ranked set.
    for index in 0..26 {
        import(
            &mut db,
            &format!("filler-{index}"),
            &format!("保留 模板段落 {index}"),
            None,
        );
    }
    for index in 0..4 {
        import(
            &mut db,
            &format!("selective-{index}"),
            &format!("保留 罕见词 {index}"),
            None,
        );
    }
    let results = db.keyword_search(&query("保留 罕见词")).unwrap();
    assert_eq!(results.items.len(), 30);
}

/// Diagnostic probe, not a gate: prints what the boilerplate rule costs and saves
/// on the source channel. Run it explicitly:
///
///     cargo test --release -p pkms-core --locked --test retrieval_sources_ranked \
///         sources_channel_pruning_cost_probe -- --ignored --nocapture
///
/// Both corpora hold 30 and 40 one-block sources and answer the same query, so the
/// only difference is whether the repeating word passes the boilerplate floor
/// (32 distinct sources): below it the body match keeps every term (the shape this
/// channel had before the rule), above it the repeating term leaves the match.
#[test]
#[ignore = "diagnostic probe: prints source-channel timings, asserts nothing"]
fn sources_channel_pruning_cost_probe() {
    use std::time::Instant;

    fn percentile(values: &mut [f64], share: f64) -> f64 {
        values.sort_by(|left, right| left.partial_cmp(right).unwrap());
        values[((values.len() as f64 - 1.0) * share).round() as usize]
    }

    fn measure(filler: usize, samples: usize) -> (usize, f64, f64) {
        let root = tempdir().unwrap();
        let mut db = Library::open(root.path()).unwrap();
        for index in 0..filler {
            import(
                &mut db,
                &format!("filler-{index}"),
                &format!("保留 模板段落 {index}"),
                None,
            );
        }
        for index in 0..4 {
            import(
                &mut db,
                &format!("selective-{index}"),
                &format!("保留 罕见词 {index}"),
                None,
            );
        }
        let request = query("保留 罕见词");
        for _ in 0..3 {
            db.keyword_search(&request).unwrap();
        }
        let mut timings = Vec::with_capacity(samples);
        let mut hits = 0;
        for _ in 0..samples {
            let started = Instant::now();
            let result = db.keyword_search(&request).unwrap();
            timings.push(started.elapsed().as_secs_f64() * 1000.0);
            hits = result.items.len();
        }
        (
            hits,
            percentile(&mut timings.clone(), 0.5),
            percentile(&mut timings, 0.95),
        )
    }

    let (below_hits, below_p50, below_p95) = measure(26, 50);
    let (above_hits, above_p50, above_p95) = measure(36, 50);
    println!(
        "sources={{30 sources, term below floor: hits={below_hits} p50={below_p50:.3} ms p95={below_p95:.3} ms}} \
         sources={{40 sources, term above floor: hits={above_hits} p50={above_p50:.3} ms p95={above_p95:.3} ms}}"
    );
}

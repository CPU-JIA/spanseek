//! Controlled cosine values exercise authorization/ranking, not embedding quality.
use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn profile(db: &mut Library, revision: &str) -> EmbeddingProfile {
    db.register_embedding_profile(
        &id(),
        EmbeddingProfileSpec {
            endpoint: None,
            model: "annotation-vector-fixture".into(),
            revision: revision.into(),
            dimensions: 3,
            normalized: true,
            query_prefix: "query: ".into(),
            passage_prefix: "passage: ".into(),
            tokenizer: "fixture".into(),
            chunker: "exact-annotation".into(),
            max_tokens: 512,
            overlap_tokens: 0,
            provider: None,
            output_dimensions: None,
        },
    )
    .unwrap()
    .profile
}
fn fixture(db: &mut Library) -> SearchQuery {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "私人标题secret".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let text = "范围外secret 授权证据 末尾secret";
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
    let block = db
        .source_text_page(&version.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    let collection = db.create_collection("摘录主题").unwrap();
    // Identical parent/range under two distinct grants must remain distinct evidence.
    for _ in 0..2 {
        let annotation = db
            .create_annotation(
                &id(),
                CreateAnnotation {
                    locator: SourceLocator::Text {
                        source_version_id: version.id.clone(),
                        block_id: block.id.clone(),
                        start_codepoint: 10,
                        end_codepoint: 14,
                        quote: "授权证据".into(),
                        prefix: "范围外secret ".into(),
                        suffix: " 末尾secret".into(),
                    },
                    comment: "评论secret不作证据".into(),
                },
            )
            .unwrap()
            .annotation
            .unwrap();
        let base = db
            .collection_get(&collection.id)
            .unwrap()
            .metadata
            .row_version;
        db.collection_add_members(
            &id(),
            ChangeCollectionMembers {
                collection_id: collection.id.clone(),
                base_row_version: base,
                member_ids: vec![annotation.id],
            },
        )
        .unwrap();
    }
    SearchQuery {
        query: "授权证据".into(),
        mode: SearchMode::Semantic,
        scope: SearchScope {
            collection_id: Some(collection.id),
            ..Default::default()
        },
        limit: 8,
        minimum_cosine: Some(0.5),
    }
}
fn candidate(
    evidence: &SearchEvidence,
    profile: &EmbeddingProfile,
    score: f64,
) -> AnnotationVectorCandidate {
    AnnotationVectorCandidate {
        evidence: evidence.clone(),
        profile_id: profile.id.clone(),
        cosine_similarity: score,
    }
}

#[test]
fn exact_annotation_vectors_work_without_generations_and_rrf_keeps_grants_separate() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let mut query = fixture(&mut db);
    let profile = profile(&mut db, "one");
    assert!(db.active_index_generation_ids().unwrap().is_empty());
    let inputs = db.annotation_search_inputs(&query.scope).unwrap();
    assert_eq!(inputs.len(), 2);
    let candidates = inputs
        .iter()
        .map(|e| candidate(e, &profile, 0.9))
        .collect::<Vec<_>>();
    let semantic = db
        .search_with_annotation_vectors(&query, vec![], candidates.clone())
        .unwrap();
    assert!(semantic.semantic_available);
    assert!(semantic.unavailable_reason.is_none());
    assert_eq!(semantic.items.len(), 2);
    for e in &semantic.items {
        assert_eq!(e.text, "授权证据");
        assert!(!e.title.contains("secret"));
        assert!(e.generation_id.is_none());
        assert_eq!(
            e.annotation
                .as_ref()
                .unwrap()
                .embedding_profile_id
                .as_deref(),
            Some(profile.id.as_str())
        );
    }
    query.mode = SearchMode::Hybrid;
    let fused = db
        .search_with_annotation_vectors(&query, vec![], candidates)
        .unwrap();
    assert_eq!(fused.items.len(), 2);
    for e in &fused.items {
        assert!(e.lexical_rank.is_some());
        assert_eq!(e.cosine_similarity, Some(0.9));
        assert!(e.fusion_rank.unwrap() > 1.0 / 61.0);
        assert_eq!(
            e.annotation
                .as_ref()
                .unwrap()
                .embedding_profile_id
                .as_deref(),
            Some(profile.id.as_str())
        );
    }
}

#[test]
fn invalid_annotation_candidates_cannot_make_semantics_available_or_escape_scope() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let query = fixture(&mut db);
    let old_profile = profile(&mut db, "old");
    let active = profile(&mut db, "active");
    let input = db.annotation_search_inputs(&query.scope).unwrap().remove(0);
    let valid = candidate(&input, &active, 0.9);
    let mut invalid = vec![candidate(&input, &old_profile, 0.99)];
    let mut outside = valid.clone();
    outside.evidence.target.start_codepoint = 0;
    outside.evidence.text = "范围外secret 授权证据".into();
    invalid.push(outside);
    let mut forged = valid.clone();
    forged.evidence.text = "伪造".into();
    invalid.push(forged);
    let mut forged = valid.clone();
    forged.evidence.generation_id = Some(id());
    invalid.push(forged);
    let mut forged = valid.clone();
    forged.evidence.annotation = None;
    invalid.push(forged);
    let mut forged = valid.clone();
    forged
        .evidence
        .annotation
        .as_mut()
        .unwrap()
        .embedding_profile_id = Some(old_profile.id);
    invalid.push(forged);
    for c in invalid {
        let result = db
            .search_with_annotation_vectors(&query, vec![], vec![c])
            .unwrap();
        assert!(result.items.is_empty());
        assert!(!result.semantic_available);
    }
    for score in [f64::NAN, f64::INFINITY, 1.01, -1.01] {
        assert!(
            db.search_with_annotation_vectors(
                &query,
                vec![],
                vec![candidate(&input, &active, score)]
            )
            .is_err()
        );
    }
    let other = db.create_collection("另一主题").unwrap();
    let mut other_query = query.clone();
    other_query.scope.collection_id = Some(other.id);
    let result = db
        .search_with_annotation_vectors(&other_query, vec![], vec![valid.clone()])
        .unwrap();
    assert!(result.items.is_empty() && !result.semantic_available);
    let mut invalid_first = valid.clone();
    invalid_first.evidence.text = "同key伪造优先".into();
    invalid_first.cosine_similarity = 1.0;
    let result = db
        .search_with_annotation_vectors(&query, vec![], vec![invalid_first, valid])
        .unwrap();
    assert_eq!(result.items.len(), 1);
}

#[test]
fn valid_annotation_below_threshold_is_available_with_no_hits() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let query = fixture(&mut db);
    let active = profile(&mut db, "active");
    let input = db.annotation_search_inputs(&query.scope).unwrap().remove(0);
    assert!(db.active_index_generation_ids().unwrap().is_empty());
    let result = db
        .search_with_annotation_vectors(&query, vec![], vec![candidate(&input, &active, 0.4)])
        .unwrap();
    assert!(result.items.is_empty());
    assert!(result.semantic_available);
    assert!(result.unavailable_reason.is_none());
    let mut forged = candidate(&input, &active, 0.4);
    forged.evidence.text = "invalid low score".into();
    let result = db
        .search_with_annotation_vectors(&query, vec![], vec![forged])
        .unwrap();
    assert!(!result.semantic_available);
}

#[test]
fn old_evidence_json_omits_optional_annotation_and_roundtrips() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("evidence", "evidence").unwrap();
    let query = SearchQuery {
        query: "evidence".into(),
        mode: SearchMode::Lexical,
        scope: SearchScope::default(),
        limit: 8,
        minimum_cosine: None,
    };
    let evidence = db.keyword_search(&query).unwrap().items.remove(0);
    assert!(evidence.annotation.is_none());
    let json = serde_json::to_value(&evidence).unwrap();
    assert!(json.get("annotation").is_none());
    let decoded: SearchEvidence = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), json);
}

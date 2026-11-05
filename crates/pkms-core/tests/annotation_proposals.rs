//! Controlled source geometry/answer text validate persistence, not model or PDF rendering.
use pkms_core::*;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn publish(
    db: &mut Library,
    format: SourceFormat,
    previous: Option<&SourceSummary>,
) -> (SourceVersion, SourceBlock) {
    let text = if previous.is_some() { "WXYZ" } else { "ABCD" };
    let bytes = if format == SourceFormat::Pdf {
        format!("%PDF-1.7 controlled {text}").into_bytes()
    } else {
        text.as_bytes().to_vec()
    };
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "controlled source".into(),
                format,
                origin_url: None,
                collection_id: None,
                add_version_to: previous.map(|s| s.id.clone()),
                base_row_version: previous.map(|s| s.row_version),
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    let parsed = if format == SourceFormat::Pdf {
        ParsedSource {
            title: None,
            source_version_id: version.id.clone(),
            parser_version: "controlled".into(),
            author: None,
            page_count: Some(1),
            needs_ocr: false,
            blocks: vec![SourceBlock {
                id: id(),
                text: text.into(),
                page_index: Some(0),
                pdf_geometry: Some(PdfBlockGeometry {
                    page_label: Some("1".into()),
                    crop_box: [0., 0., 100., 100.],
                    rotation: 0,
                    coordinate_space: "pdf_user_space".into(),
                    chars: (0..4)
                        .map(|i| PdfCharacterBox {
                            start_codepoint: i,
                            end_codepoint: i + 1,
                            rect: Some([f64::from(i) * 10., 0., f64::from(i + 1) * 10., 10.]),
                        })
                        .collect(),
                }),
            }],
            toc: Vec::new(),
            cover: None,
            images: Vec::new(),
        }
    } else {
        parse_text_source(&version.id, format, text.as_bytes()).unwrap()
    };
    let block = parsed.blocks[0].clone();
    db.publish_source_parse(&job.job_id, &lease.lease_token, parsed)
        .unwrap();
    (version, block)
}

#[test]
fn exact_annotation_answer_proposal_reference_keeps_text_or_pdf_locator_through_apply_and_restore()
{
    for format in [SourceFormat::Text, SourceFormat::Pdf] {
        let root = tempdir().unwrap();
        let mut db = Library::open(root.path()).unwrap();
        let (version, block) = publish(&mut db, format, None);
        let locator = if format == SourceFormat::Pdf {
            SourceLocator::Pdf {
                source_version_id: version.id.clone(),
                page_index: 0,
                page_label: Some("1".into()),
                crop_box: [0., 0., 100., 100.],
                rotation: 0,
                coordinate_space: "pdf_user_space".into(),
                rects: vec![[10., 0., 30., 10.]],
                quote: "BC".into(),
                prefix: "A".into(),
                suffix: "D".into(),
            }
        } else {
            SourceLocator::Text {
                source_version_id: version.id.clone(),
                block_id: block.id,
                start_codepoint: 1,
                end_codepoint: 3,
                quote: "BC".into(),
                prefix: "A".into(),
                suffix: "D".into(),
            }
        };
        let annotation = db
            .create_annotation(
                &id(),
                CreateAnnotation {
                    locator: locator.clone(),
                    comment: "".into(),
                },
            )
            .unwrap()
            .annotation
            .unwrap();
        let accepted = db
            .accept_answer(
                &id(),
                StartAnswer {
                    max_output_tokens: None,
                    citation_count: None,
                    agent: None,
                    search: SearchQuery {
                        query: "BC".into(),
                        mode: SearchMode::Lexical,
                        scope: SearchScope {
                            object_ids: vec![annotation.id.clone()],
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
            .unwrap();
        let lease = db.claim_answer(&accepted.answer.id).unwrap().unwrap();
        let frozen = db
            .prepare_answer_evidence(&accepted.answer.id, &lease.lease_token, vec![])
            .unwrap();
        assert_eq!(frozen.evidence.len(), 1);
        let evidence = &frozen.evidence[0].evidence;
        assert_eq!(evidence.text, "BC");
        let grant = evidence.annotation.as_ref().unwrap();
        let document = serde_json::from_value(json!({"type":"doc","content":[
            {"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"BC [E1]"}]},
            {"type":"paragraph","attrs":{"id":id()},"content":[{"type":"noteReference","attrs":{
                "target_id":grant.annotation_id,"target_revision_id":evidence.target.content_revision,"occurrence_id":id(),"label":"[E1] 摘录"
            }}]}
        ]})).unwrap();
        db.complete_answer(CompleteAnswer {
            answer_id: frozen.id.clone(),
            lease_token: lease.lease_token,
            text: "BC [E1]".into(),
            cited_ids: vec!["E1".into()],
        })
        .unwrap();
        let proposal = db
            .create_proposal(
                &id(),
                CreateProposal {
                    chat_run_id: None,
                    external_request_id: None,
                    answer_id: Some(frozen.id),
                    note_id: None,
                    base_revision_id: None,
                    title: "研究输出".into(),
                    document_schema: 2,
                    document,
                },
            )
            .unwrap()
            .proposal;
        let command = id();
        let note = db.apply_proposal(&command, &proposal.id).unwrap().note;
        assert_eq!(
            db.apply_proposal(&command, &proposal.id).unwrap().note.id,
            note.id
        );
        let references = db.note_references(&note.id, &note.revision_id).unwrap();
        assert_eq!(references.len(), 1);
        let reference = &references[0];
        assert_eq!(reference.target_id, annotation.id);
        assert_eq!(
            reference.target_revision_id.as_deref(),
            Some(version.id.as_str())
        );
        assert!(reference.target_block_id.is_none());
        assert_eq!(
            db.body_backlinks(&annotation.id).unwrap()[0].source_id,
            note.id
        );
        let target = ResolveLink {
            target_id: reference.target_id.clone(),
            target_revision_id: reference.target_revision_id.clone(),
            target_block_id: None,
        };
        let resolved = db.resolve_link(target.clone()).unwrap();
        assert_eq!(resolved.state, LinkState::Available);
        let jump = resolved.source_target.unwrap();
        assert_eq!(jump.annotation_id.as_deref(), Some(annotation.id.as_str()));
        assert!(matches!(jump.locator_status, Some(LocatorStatus::Exact)));
        assert_eq!(
            serde_json::to_value(jump.locator).unwrap(),
            serde_json::to_value(&locator).unwrap()
        );
        let previous = db.source(&version.source_id).unwrap();
        let (later, _) = publish(&mut db, format, Some(&previous));
        assert_ne!(later.id, version.id);
        assert_eq!(
            db.resolve_link(target.clone())
                .unwrap()
                .source_target
                .unwrap()
                .source_version_id,
            version.id
        );
        db.set_annotation_trashed(
            &id(),
            SetObjectFlag {
                object_id: annotation.id.clone(),
                base_row_version: annotation.row_version,
                value: true,
            },
        )
        .unwrap();
        assert_eq!(
            db.resolve_link(target.clone()).unwrap().state,
            LinkState::TargetTrashed
        );
        let package = root.path().join("proposal.zip");
        db.create_backup(&package, &mut |_| Ok(())).unwrap();
        let destination = root.path().join("restored");
        prepare_restore(&package, &destination, &mut |_| Ok(()))
            .unwrap()
            .publish()
            .unwrap();
        let restored = Library::open_existing(destination).unwrap();
        assert_eq!(
            restored
                .note_references(&note.id, &note.revision_id)
                .unwrap()[0]
                .target_id,
            annotation.id
        );
        let restored_jump = restored
            .resolve_link(target)
            .unwrap()
            .source_target
            .unwrap();
        assert_eq!(restored_jump.source_version_id, version.id);
        assert_eq!(
            serde_json::to_value(restored_jump.locator).unwrap(),
            serde_json::to_value(locator).unwrap()
        );
    }
}

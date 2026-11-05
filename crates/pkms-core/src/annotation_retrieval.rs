//! Collection/explicit annotation grants authorize slices, never their parent document.
use crate::*;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

const MAX_ANNOTATIONS: usize = 4096;
const MAX_SLICES: usize = 16384;
const MAX_QUOTE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESOLUTION_BYTES: usize = 64 * 1024 * 1024;
const TITLE: &str = "摘录";

fn validate_scope(scope: &SearchScope) -> Result<()> {
    if scope.object_ids.len() > 100 || scope.tag_ids.len() > 100 || scope.kinds.len() > 2 {
        return Err(Error::InvalidInput("annotation scope exceeds bounds"));
    }
    for id in scope
        .object_ids
        .iter()
        .chain(&scope.tag_ids)
        .chain(scope.collection_id.iter())
    {
        crate::document::valid_uuid(id)?;
    }
    Ok(())
}
fn enum_value<T: serde::Serialize>(value: Option<T>) -> Result<Option<String>> {
    value
        .map(|v| {
            serde_json::to_value(v)?
                .as_str()
                .map(str::to_owned)
                .ok_or(Error::IntegrityFailure)
        })
        .transpose()
}
fn allowed_ids(db: &Connection, scope: &SearchScope) -> Result<Vec<String>> {
    validate_scope(scope)?;
    if (scope.collection_id.is_none() && scope.object_ids.is_empty())
        || (!scope.kinds.is_empty() && !scope.kinds.contains(&EvidenceKind::Source))
    {
        return Ok(vec![]);
    }
    let ids=db.prepare("SELECT a.id FROM annotations a JOIN objects o ON o.id=a.id JOIN source_versions v ON v.id=a.source_version_id JOIN sources s ON s.id=v.source_id JOIN objects parent ON parent.id=s.id LEFT JOIN reading_states r ON r.source_id=s.id WHERE o.trashed=0 AND o.search_enabled=1 AND parent.trashed=0 AND parent.search_enabled=1 AND (json_array_length(?1)=0 OR a.id IN (SELECT value FROM json_each(?1))) AND (?2 IS NULL OR EXISTS(SELECT 1 FROM memberships m JOIN objects c ON c.id=m.collection_id WHERE m.member_id=a.id AND m.collection_id=?2 AND c.trashed=0)) AND NOT EXISTS(SELECT 1 FROM json_each(?3) t WHERE NOT EXISTS(SELECT 1 FROM object_tags ot WHERE ot.object_id=a.id AND ot.tag_id=t.value)) AND (?4 IS NULL OR o.archived=?4) AND (?5 IS NULL OR v.format=?5) AND (?6 IS NULL OR coalesce(r.status,'unread')=?6) AND (?7 IS NULL OR o.inbox_state=?7) ORDER BY a.id LIMIT ?8")?.query_map(params![serde_json::to_string(&scope.object_ids)?,scope.collection_id,serde_json::to_string(&scope.tag_ids)?,scope.archived,enum_value(scope.source_format)?,enum_value(scope.reading_status.as_ref())?,scope.inbox_state.map(|s|s.as_str()),(MAX_ANNOTATIONS+1) as u32],|r|r.get(0))?.collect::<std::result::Result<Vec<String>,_>>()?;
    if ids.len() > MAX_ANNOTATIONS {
        return Err(Error::InvalidInput("annotation scope exceeds 4096 entries"));
    }
    Ok(ids)
}
fn locator_hash(locator: &SourceLocator) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(locator)?)
    ))
}
fn slices(
    db: &Connection,
    annotation: &Annotation,
    parse: Option<&str>,
) -> Result<Vec<crate::source_annotations::LocatorSlice>> {
    match &annotation.locator {
        SourceLocator::Text {
            source_version_id,
            block_id,
            start_codepoint,
            end_codepoint,
            quote,
            ..
        } => {
            crate::source_annotations::validate_locator(db, &annotation.locator)?;
            let parse_id:Option<String>=db.query_row("SELECT b.parse_id FROM source_blocks b JOIN source_parses p ON p.id=b.parse_id WHERE b.id=?1 AND p.source_version_id=?2 AND (?3 IS NULL OR p.id=?3) AND NOT EXISTS(SELECT 1 FROM source_parse_staging z WHERE z.parse_id=p.id)",params![block_id,source_version_id,parse],|r|r.get(0)).optional()?;
            let Some(parse_id) = parse_id else {
                return Ok(vec![]);
            };
            if start_codepoint >= end_codepoint {
                return Ok(vec![]);
            }
            Ok(vec![crate::source_annotations::LocatorSlice {
                block_id: block_id.clone(),
                parse_id,
                page_index: None,
                start_codepoint: *start_codepoint,
                end_codepoint: *end_codepoint,
                text: quote.clone(),
            }])
        }
        SourceLocator::Pdf { .. } => {
            Ok(
                crate::source_annotations::pdf_evidence_slices(db, &annotation.locator, parse)?
                    .unwrap_or_default(),
            )
        }
    }
}
fn evidence_id(annotation: &Annotation, hash: &str, target: &EvidenceTarget) -> Result<String> {
    Ok(format!(
        "annotation:{:x}",
        Sha256::digest(serde_json::to_vec(&(
            annotation.id.as_str(),
            annotation.row_version,
            hash,
            target
        ))?)
    ))
}
impl Library {
    pub fn annotation_search_inputs(&self, scope: &SearchScope) -> Result<Vec<SearchEvidence>> {
        let _snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let ids = allowed_ids(&self.connection, scope)?;
        let mut output = vec![];
        let mut bytes = 0usize;
        let mut resolution_bytes = 0usize;
        for id in ids {
            let annotation = crate::source_annotations::read_annotation(&self.connection, &id)?;
            let (source,metadata):(String,u32)=self.connection.query_row("SELECT v.source_id,o.row_version FROM source_versions v JOIN objects o ON o.id=v.source_id WHERE v.id=?1",[&annotation.source_version_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let work_bytes:i64=match &annotation.locator {
                SourceLocator::Text{block_id,..}=>self.connection.query_row("SELECT length(CAST(text AS BLOB)) FROM source_blocks WHERE id=?1",[block_id],|r|r.get(0)).optional()?.unwrap_or(0),
                SourceLocator::Pdf{source_version_id,page_index,..}=>self.connection.query_row("SELECT coalesce(sum(length(CAST(b.text AS BLOB))+coalesce(length(CAST(b.geometry_json AS BLOB)),0)),0) FROM source_blocks b JOIN source_active_parses a ON a.parse_id=b.parse_id WHERE a.source_version_id=?1 AND b.page_index=?2",params![source_version_id,page_index],|r|r.get(0))?,
            };
            resolution_bytes = resolution_bytes
                .checked_add(usize::try_from(work_bytes).map_err(|_| Error::IntegrityFailure)?)
                .ok_or(Error::InvalidInput("annotation resolution exceeds bounds"))?;
            if resolution_bytes > MAX_RESOLUTION_BYTES {
                return Err(Error::InvalidInput("annotation resolution exceeds 64 MiB"));
            }
            let hash = locator_hash(&annotation.locator)?;
            for slice in slices(&self.connection, &annotation, None)? {
                bytes = bytes
                    .checked_add(slice.text.len())
                    .ok_or(Error::InvalidInput("annotation quotes exceed bounds"))?;
                if bytes > MAX_QUOTE_BYTES || output.len() >= MAX_SLICES {
                    return Err(Error::InvalidInput("annotation evidence exceeds bounds"));
                }
                let target = EvidenceTarget {
                    object_id: source.clone(),
                    kind: EvidenceKind::Source,
                    content_revision: annotation.source_version_id.clone(),
                    metadata_revision: metadata,
                    parse_id: Some(slice.parse_id),
                    block_id: slice.block_id,
                    start_codepoint: slice.start_codepoint,
                    end_codepoint: slice.end_codepoint,
                    page_index: slice.page_index,
                };
                output.push(SearchEvidence {
                    id: evidence_id(&annotation, &hash, &target)?,
                    target,
                    title: TITLE.into(),
                    text: slice.text,
                    lexical_rank: None,
                    cosine_similarity: None,
                    fusion_rank: None,
                    generation_id: None,
                    annotation: Some(AnnotationEvidence {
                        annotation_id: annotation.id.clone(),
                        annotation_revision: annotation.row_version,
                        locator_hash: hash.clone(),
                        embedding_profile_id: None,
                    }),
                });
            }
        }
        Ok(output)
    }
}

/// Historical evidence remains a valid fact after membership/lifecycle changes.
/// The request's immutable object/type/format restrictions still bind that fact.
/// Live evidence additionally requires the exact grant and current parent metadata.
pub(crate) fn validate_annotation_evidence(
    db: &Connection,
    scope: &SearchScope,
    evidence: &SearchEvidence,
    history: bool,
) -> Result<bool> {
    let Some(binding) = &evidence.annotation else {
        return Ok(false);
    };
    validate_scope(scope)?;
    if crate::document::valid_uuid(&binding.annotation_id).is_err()
        || binding.annotation_revision == 0
        || evidence.target.metadata_revision == 0
        || evidence.target.kind != EvidenceKind::Source
        || (!scope.kinds.is_empty() && !scope.kinds.contains(&EvidenceKind::Source))
        || (!scope.object_ids.is_empty() && !scope.object_ids.contains(&binding.annotation_id))
        || (scope.collection_id.is_none() && scope.object_ids.is_empty())
        || evidence.title != TITLE
        || evidence.generation_id.is_some()
    {
        return Ok(false);
    }
    let annotation = match crate::source_annotations::read_annotation(db, &binding.annotation_id) {
        Ok(a) => a,
        Err(Error::NotFound) => return Ok(false),
        Err(e) => return Err(e),
    };
    if annotation.row_version < binding.annotation_revision
        || (!history && annotation.row_version != binding.annotation_revision)
        || locator_hash(&annotation.locator)? != binding.locator_hash
        || annotation.source_version_id != evidence.target.content_revision
    {
        return Ok(false);
    }
    let (owner,metadata,format):(String,u32,String)=db.query_row("SELECT v.source_id,o.row_version,v.format FROM source_versions v JOIN objects o ON o.id=v.source_id WHERE v.id=?1",[&annotation.source_version_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    if owner != evidence.target.object_id
        || evidence.target.metadata_revision > metadata
        || (!history && evidence.target.metadata_revision != metadata)
        || enum_value(scope.source_format)?.is_some_and(|required| required != format)
    {
        return Ok(false);
    }
    if !history {
        let mut narrowed = scope.clone();
        narrowed.object_ids = vec![binding.annotation_id.clone()];
        if allowed_ids(db, &narrowed)?.is_empty() {
            return Ok(false);
        }
    }
    if binding.embedding_profile_id.is_some() != evidence.cosine_similarity.is_some() {
        return Ok(false);
    }
    if let Some(profile) = &binding.embedding_profile_id {
        let valid: bool = if history {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM embedding_profiles WHERE id=?1)",
                [profile],
                |r| r.get(0),
            )?
        } else {
            db.query_row("SELECT EXISTS(SELECT 1 FROM search_settings WHERE singleton=1 AND active_profile_id=?1)",[profile],|r|r.get(0))?
        };
        if !valid {
            return Ok(false);
        }
    }
    if evidence
        .cosine_similarity
        .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
    {
        return Ok(false);
    }
    let t = &evidence.target;
    let Some(parse) = t.parse_id.as_deref() else {
        return Ok(false);
    };
    let ranges = match slices(db, &annotation, history.then_some(parse)) {
        Ok(r) => r,
        Err(Error::NotFound | Error::InvalidInput(_)) => return Ok(false),
        Err(e) => return Err(e),
    };
    Ok(ranges.into_iter().any(|s| {
        if s.block_id != t.block_id
            || s.parse_id != parse
            || s.page_index != t.page_index
            || t.end_codepoint > s.end_codepoint
            || t.start_codepoint < s.start_codepoint
            || t.start_codepoint >= t.end_codepoint
        {
            return false;
        }
        s.text
            .chars()
            .skip((t.start_codepoint - s.start_codepoint) as usize)
            .take((t.end_codepoint - t.start_codepoint) as usize)
            .collect::<String>()
            == evidence.text
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(db: &mut Library) -> (Annotation, SearchScope) {
        let id = || Uuid::new_v4().to_string();
        let command = id();
        let job = db
            .accept_source_import(
                &command,
                ImportSource {
                    input_token: command.clone(),
                    original_name: "private".into(),
                    format: SourceFormat::Text,
                    origin_url: None,
                    collection_id: None,
                    add_version_to: None,
                    base_row_version: None,
                },
            )
            .unwrap();
        let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
        let bytes = b"secretQUOTEhidden";
        let v = db
            .capture_source_original(
                &job.job_id,
                &lease.lease_token,
                &mut std::io::Cursor::new(bytes),
                &mut |_| Ok(()),
            )
            .unwrap();
        let parsed = parse_text_source(&v.id, SourceFormat::Text, bytes).unwrap();
        let block = parsed.blocks[0].id.clone();
        db.publish_source_parse(&job.job_id, &lease.lease_token, parsed)
            .unwrap();
        let a = db
            .create_annotation(
                &id(),
                CreateAnnotation {
                    locator: SourceLocator::Text {
                        source_version_id: v.id,
                        block_id: block,
                        start_codepoint: 6,
                        end_codepoint: 11,
                        quote: "QUOTE".into(),
                        prefix: "secret".into(),
                        suffix: "hidden".into(),
                    },
                    comment: "comment".into(),
                },
            )
            .unwrap()
            .annotation
            .unwrap();
        let q = SearchScope {
            object_ids: vec![a.id.clone()],
            ..Default::default()
        };
        (a, q)
    }
    #[test]
    fn evidence_rejects_expansion_forgery_and_live_revocation_but_preserves_history() {
        let root = tempfile::tempdir().unwrap();
        let mut db = Library::open(root.path()).unwrap();
        let (a, q) = fixture(&mut db);
        let e = db.annotation_search_inputs(&q).unwrap().remove(0);
        assert!(validate_annotation_evidence(&db.connection, &q, &e, false).unwrap());
        let mut forged = e.clone();
        forged.target.start_codepoint = 0;
        forged.text = "secretQUOTE".into();
        assert!(!validate_annotation_evidence(&db.connection, &q, &forged, false).unwrap());
        forged = e.clone();
        forged.text = "OTHER".into();
        assert!(!validate_annotation_evidence(&db.connection, &q, &forged, true).unwrap());
        forged = e.clone();
        forged.annotation.as_mut().unwrap().locator_hash = "wrong".into();
        assert!(!validate_annotation_evidence(&db.connection, &q, &forged, true).unwrap());
        forged = e.clone();
        forged.cosine_similarity = Some(0.8);
        assert!(!validate_annotation_evidence(&db.connection, &q, &forged, false).unwrap());
        assert!(
            !validate_annotation_evidence(&db.connection, &SearchScope::default(), &e, false)
                .unwrap()
        );
        db.connection.execute("UPDATE objects SET trashed=1,search_enabled=0,row_version=row_version+1 WHERE id=?1",[&a.id]).unwrap();
        db.connection
            .execute(
                "UPDATE objects SET trashed=1,row_version=row_version+1 WHERE id=?1",
                [&e.target.object_id],
            )
            .unwrap();
        assert!(!validate_annotation_evidence(&db.connection, &q, &e, false).unwrap());
        assert!(validate_annotation_evidence(&db.connection, &q, &e, true).unwrap());
        forged = e.clone();
        forged.annotation.as_mut().unwrap().annotation_revision = 1000;
        assert!(!validate_annotation_evidence(&db.connection, &q, &forged, true).unwrap());
    }
    #[test]
    fn history_keeps_explicit_annotation_grant_even_for_identical_quotes() {
        let root = tempfile::tempdir().unwrap();
        let mut db = Library::open(root.path()).unwrap();
        let (a, scope_a) = fixture(&mut db);
        let b = db
            .create_annotation(
                &Uuid::new_v4().to_string(),
                CreateAnnotation {
                    locator: a.locator.clone(),
                    comment: "another independent annotation".into(),
                },
            )
            .unwrap()
            .annotation
            .unwrap();
        let evidence_a = db.annotation_search_inputs(&scope_a).unwrap().remove(0);
        let scope_b = SearchScope {
            object_ids: vec![b.id],
            ..Default::default()
        };
        let evidence_b = db.annotation_search_inputs(&scope_b).unwrap().remove(0);
        assert_eq!(evidence_a.text, evidence_b.text);
        assert_eq!(
            serde_json::to_value(&evidence_a.target).unwrap(),
            serde_json::to_value(&evidence_b.target).unwrap()
        );
        assert!(validate_annotation_evidence(&db.connection, &scope_a, &evidence_a, true).unwrap());
        assert!(validate_annotation_evidence(&db.connection, &scope_b, &evidence_b, true).unwrap());
        // A valid locator, identical target and authentic B binding cannot replace
        // the A grant frozen into the answer request.
        assert!(
            !validate_annotation_evidence(&db.connection, &scope_a, &evidence_b, true).unwrap()
        );
    }
    #[test]
    fn history_keeps_static_scope_kind_format_and_grant_restrictions() {
        let root = tempfile::tempdir().unwrap();
        let mut db = Library::open(root.path()).unwrap();
        let (_, scope) = fixture(&mut db);
        let evidence = db.annotation_search_inputs(&scope).unwrap().remove(0);
        let matching = SearchScope {
            kinds: vec![EvidenceKind::Source],
            source_format: Some(SourceFormat::Text),
            ..scope.clone()
        };
        assert!(validate_annotation_evidence(&db.connection, &matching, &evidence, true).unwrap());
        for rejected in [
            SearchScope::default(),
            SearchScope {
                object_ids: vec![evidence.target.object_id.clone()],
                ..Default::default()
            },
            SearchScope {
                kinds: vec![EvidenceKind::Note],
                ..scope.clone()
            },
            SearchScope {
                source_format: Some(SourceFormat::Pdf),
                ..scope.clone()
            },
        ] {
            assert!(
                !validate_annotation_evidence(&db.connection, &rejected, &evidence, true).unwrap()
            );
        }
    }
    #[test]
    fn oversized_annotation_collection_errors_instead_of_returning_first_page() {
        let root = tempfile::tempdir().unwrap();
        let mut db = Library::open(root.path()).unwrap();
        let (a, _) = fixture(&mut db);
        let collection = db.create_collection("large").unwrap();
        let tx = db.connection.unchecked_transaction().unwrap();
        for position in 0..=MAX_ANNOTATIONS {
            let id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO objects(id,kind) VALUES(?1,'annotation')",
                [&id],
            )
            .unwrap();
            tx.execute("INSERT INTO annotations(id,source_version_id,locator_schema,locator_json,comment) SELECT ?1,source_version_id,locator_schema,locator_json,'' FROM annotations WHERE id=?2",params![id,a.id]).unwrap();
            tx.execute(
                "INSERT INTO memberships VALUES(?1,?2,?3)",
                params![collection.id, id, position as u32],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        assert!(matches!(
            db.annotation_search_inputs(&SearchScope {
                collection_id: Some(collection.id),
                ..Default::default()
            }),
            Err(Error::InvalidInput("annotation scope exceeds 4096 entries"))
        ));
    }
}

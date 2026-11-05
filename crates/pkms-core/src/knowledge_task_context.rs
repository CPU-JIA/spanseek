//! Explicit object selections become checked, bounded evidence; coverage never implies full reading.
use crate::*;
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeSet;

const MAX_FRAGMENTS: usize = 32;
const MAX_BYTES: usize = 256 * 1024;

/// Avoid cutting a larger first object to a fair share when the complete selection fits.
/// This bounded preflight stops as soon as either global limit is exceeded.
fn selection_fits(library: &Library, targets: &[KnowledgeTaskMaterialTarget]) -> Result<bool> {
    let mut bytes = 0usize;
    let mut fragments = 0usize;
    for target in targets {
        let kind: Option<String> = library
            .connection
            .query_row(
                "SELECT kind FROM objects WHERE id=?1",
                [&target.object_id],
                |r| r.get(0),
            )
            .optional()?;
        let texts = match kind.as_deref() {
            Some("annotation") => {
                let evidence = library.annotation_search_inputs(&SearchScope {
                    object_ids: vec![target.object_id.clone()],
                    ..Default::default()
                })?;
                if target.content_revision.as_ref().is_some_and(|revision| {
                    evidence
                        .iter()
                        .any(|e| &e.target.content_revision != revision)
                }) {
                    continue;
                }
                evidence.into_iter().map(|e| e.text).collect::<Vec<_>>()
            }
            Some("note" | "source") => {
                let input = match crate::retrieval::read_index_input(
                    &library.connection,
                    &target.object_id,
                ) {
                    Ok(input) => input,
                    Err(Error::NotFound) => continue,
                    Err(error) => return Err(error),
                };
                if target
                    .content_revision
                    .as_ref()
                    .is_some_and(|revision| revision != &input.content_revision)
                {
                    continue;
                }
                input.blocks.into_iter().map(|b| b.text).collect::<Vec<_>>()
            }
            _ => continue,
        };
        for text in texts {
            if text.is_empty() {
                continue;
            }
            if text.len() > MAX_BYTES - bytes || fragments == MAX_FRAGMENTS {
                return Ok(false);
            }
            bytes += text.len();
            fragments += 1;
        }
    }
    Ok(true)
}

impl Library {
    pub fn knowledge_task_materials(
        &self,
        request: KnowledgeTaskMaterialsRequest,
    ) -> Result<KnowledgeTaskMaterials> {
        if request.targets.is_empty() || request.targets.len() > 100 {
            return Err(Error::InvalidInput("knowledge_task_selection_invalid"));
        }
        let mut unique = BTreeSet::new();
        for target in &request.targets {
            document::valid_uuid(&target.object_id)?;
            if let Some(revision) = &target.content_revision {
                document::valid_uuid(revision)?;
            }
            if !unique.insert(&target.object_id) {
                return Err(Error::InvalidInput("knowledge_task_selection_invalid"));
            }
        }
        let snapshot = self.connection.unchecked_transaction()?;
        let all_fit = selection_fits(self, &request.targets)?;
        let mut result = KnowledgeTaskMaterials {
            materials: vec![],
            coverage: vec![],
        };
        let mut bytes = 0usize;
        for (index, selected) in request.targets.iter().enumerate() {
            use KnowledgeTaskMaterialStatus as Status;
            let row: Option<(String, bool, bool, String)> = snapshot.query_row(
                "SELECT o.kind,o.trashed,o.search_enabled,coalesce(n.title,s.display_title,'') FROM objects o LEFT JOIN notes n ON n.id=o.id LEFT JOIN sources s ON s.id=o.id WHERE o.id=?1",
                [&selected.object_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
            ).optional()?;
            let mut coverage = KnowledgeTaskMaterialCoverage {
                object_id: selected.object_id.clone(),
                title: row.as_ref().map_or_else(String::new, |r| r.3.clone()),
                status: Status::Complete,
                total_characters: None,
                included_characters: 0,
                included_fragments: 0,
            };
            let Some((kind, trashed, enabled, _)) = row else {
                coverage.status = Status::Missing;
                result.coverage.push(coverage);
                continue;
            };
            if trashed {
                coverage.status = Status::Trashed;
            } else if !enabled {
                coverage.status = Status::SearchDisabled;
            } else if !matches!(kind.as_str(), "note" | "source" | "annotation") {
                coverage.status = Status::Unsupported;
            }
            if coverage.status != Status::Complete {
                result.coverage.push(coverage);
                continue;
            }
            // Do not load another potentially large document after the shared budget is exhausted.
            if bytes >= MAX_BYTES || result.materials.len() >= MAX_FRAGMENTS {
                coverage.status = Status::Limited;
                result.coverage.push(coverage);
                continue;
            }
            let candidates = if kind == "annotation" {
                let candidates = self.annotation_search_inputs(&SearchScope {
                    object_ids: vec![selected.object_id.clone()],
                    ..Default::default()
                })?;
                if let Some(first) = candidates.first() {
                    coverage.title = first.title.clone();
                }
                candidates
            } else {
                let input = match crate::retrieval::read_index_input(&snapshot, &selected.object_id)
                {
                    Ok(input) => input,
                    Err(Error::NotFound) => {
                        coverage.status = if kind == "source" {
                            Status::Unparsed
                        } else {
                            Status::Missing
                        };
                        result.coverage.push(coverage);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                input
                    .blocks
                    .into_iter()
                    .map(|block| SearchEvidence {
                        id: format!(
                            "material:{}:{}:{}:{}",
                            block.target.object_id,
                            block.target.content_revision,
                            block.target.block_id,
                            block.target.start_codepoint
                        ),
                        target: block.target,
                        title: input.title.clone(),
                        text: block.text,
                        lexical_rank: None,
                        cosine_similarity: None,
                        fusion_rank: None,
                        generation_id: None,
                        annotation: None,
                    })
                    .collect::<Vec<_>>()
            };
            if selected.content_revision.as_ref().is_some_and(|revision| {
                candidates
                    .iter()
                    .any(|e| &e.target.content_revision != revision)
            }) {
                coverage.status = Status::Changed;
                result.coverage.push(coverage);
                continue;
            }
            let total = candidates
                .iter()
                .map(|e| e.text.chars().count())
                .sum::<usize>();
            coverage.total_characters =
                Some(u32::try_from(total).map_err(|_| Error::IntegrityFailure)?);
            if total == 0 {
                coverage.status = Status::Empty;
                result.coverage.push(coverage);
                continue;
            }
            // Reserve room for later selected objects; any partial coverage is explicitly reported.
            let remaining_objects = if all_fit {
                1
            } else {
                request.targets.len() - index
            };
            let fragment_budget =
                ((MAX_FRAGMENTS - result.materials.len()) / remaining_objects).max(1);
            let byte_budget = ((MAX_BYTES - bytes) / remaining_objects).max(1);
            let mut object_bytes = 0usize;
            for mut evidence in candidates {
                if evidence.text.is_empty() {
                    continue;
                }
                if coverage.included_fragments as usize >= fragment_budget
                    || object_bytes >= byte_budget
                {
                    break;
                }
                let remaining = byte_budget - object_bytes;
                if evidence.text.len() > remaining {
                    // Annotation handles describe their complete resolved slice; never alter it.
                    if evidence.annotation.is_some() {
                        break;
                    }
                    let mut end = remaining.min(evidence.text.len());
                    while !evidence.text.is_char_boundary(end) {
                        end -= 1;
                    }
                    evidence.text.truncate(end);
                    evidence.target.end_codepoint =
                        evidence.target.start_codepoint + evidence.text.chars().count() as u32;
                }
                if evidence.text.is_empty() {
                    break;
                }
                if evidence.annotation.is_none() {
                    evidence.id = format!(
                        "material:{}:{}:{}:{}:{}:{}",
                        evidence.target.object_id,
                        evidence.target.content_revision,
                        evidence.target.metadata_revision,
                        evidence.target.block_id,
                        evidence.target.start_codepoint,
                        evidence.target.end_codepoint
                    );
                }
                object_bytes += evidence.text.len();
                coverage.included_characters += evidence.text.chars().count() as u32;
                coverage.included_fragments += 1;
                result.materials.push(evidence);
            }
            bytes += object_bytes;
            if coverage.included_characters != coverage.total_characters.unwrap_or(0) {
                coverage.status = Status::Limited;
            }
            result.coverage.push(coverage);
        }
        Ok(result)
    }

    pub fn knowledge_task_origins(
        &self,
        request: KnowledgeTaskOriginsRequest,
    ) -> Result<KnowledgeTaskOriginsPage> {
        document::valid_uuid(&request.note_id)?;
        collections::validate_limit(request.limit)?;
        if request.page == 0 {
            return Err(Error::InvalidInput("knowledge_task_page_invalid"));
        }
        let snapshot = self.connection.unchecked_transaction()?;
        if !snapshot.query_row(
            "SELECT EXISTS(SELECT 1 FROM notes WHERE id=?1)",
            [&request.note_id],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::NotFound);
        }
        let total = snapshot.query_row("SELECT count(*) FROM knowledge_task_outputs WHERE applied_note_id=?1 AND status='applied'",[&request.note_id],|r|r.get::<_,u32>(0))?;
        let page = request.page.min(total.max(1).div_ceil(request.limit));
        let items = snapshot.prepare("SELECT t.id,t.title,o.applied_revision_id,t.created_at,json_array_length(t.input_json,'$.materials') FROM knowledge_task_outputs o JOIN knowledge_tasks t ON t.id=o.task_id WHERE o.applied_note_id=?1 AND o.status='applied' ORDER BY o.created_at DESC,o.id DESC LIMIT ?2 OFFSET ?3")?
            .query_map(params![request.note_id,request.limit,i64::from(page-1)*i64::from(request.limit)],|r|Ok(KnowledgeTaskOrigin { task_id:r.get(0)?, task_title:r.get(1)?, applied_revision_id:r.get(2)?, task_created_at:r.get(3)?, materials_count:r.get(4)? }))?
            .collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(KnowledgeTaskOriginsPage {
            items,
            total,
            page,
            limit: request.limit,
        })
    }
}

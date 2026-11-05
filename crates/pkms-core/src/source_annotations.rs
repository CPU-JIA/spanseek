use crate::{
    Annotation, CreateAnnotation, Error, Library, LocatorStatus, ReadingState, ReadingStatus,
    ResolvedAnnotation, Result, SetObjectFlag, SetReadingState, SourceLocator, SourceMutation,
    UpdateAnnotation, document::valid_uuid, mutations,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub(crate) fn read_annotation(db: &Connection, id: &str) -> Result<Annotation> {
    valid_uuid(id)?;
    let row=db.query_row("SELECT a.source_version_id,a.locator_schema,a.locator_json,a.comment,o.row_version,o.trashed FROM annotations a JOIN objects o ON o.id=a.id WHERE a.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get::<_,String>(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?.ok_or(Error::NotFound)?;
    Ok(Annotation {
        id: id.into(),
        source_version_id: row.0,
        locator_schema: row.1,
        locator: serde_json::from_str(&row.2)?,
        comment: row.3,
        row_version: row.4,
        trashed: row.5,
    })
}
pub(crate) fn validate_locator(db: &Connection, locator: &SourceLocator) -> Result<()> {
    validate_locator_with_history(db, locator, false)
}

/// Backup validation retains immutable locators after an active parser changes.
/// This does not grant live access or require approximate PDF quotes to resolve.
pub(crate) fn validate_historical_locator(db: &Connection, locator: &SourceLocator) -> Result<()> {
    validate_locator_with_history(db, locator, true)
}

fn validate_locator_with_history(
    db: &Connection,
    locator: &SourceLocator,
    history: bool,
) -> Result<()> {
    valid_uuid(locator.source_version_id())?;
    let version = crate::sources::read_version(db, locator.source_version_id())?;
    if locator.quote().len() > 64 * 1024 || serde_json::to_vec(locator)?.len() > 128 * 1024 {
        return Err(Error::InvalidInput("locator exceeds size limit"));
    }
    match locator {
        SourceLocator::Text {
            block_id,
            start_codepoint,
            end_codepoint,
            quote,
            prefix,
            suffix,
            ..
        } => {
            if version.format == crate::SourceFormat::Pdf {
                return Err(Error::InvalidInput("PDF source requires PDF locator"));
            }
            valid_uuid(block_id)?;
            reject_staging_locator(db, locator)?;
            let text:String=db.query_row("SELECT b.text FROM source_blocks b JOIN source_parses p ON p.id=b.parse_id WHERE b.id=?1 AND p.source_version_id=?2",params![block_id,version.id],|r|r.get(0)).optional()?.ok_or(Error::NotFound)?;
            let count = text.chars().count();
            if start_codepoint > end_codepoint || *end_codepoint as usize > count {
                return Err(Error::InvalidInput("text locator offset out of bounds"));
            }
            let selected = text
                .chars()
                .skip(*start_codepoint as usize)
                .take((end_codepoint - start_codepoint) as usize)
                .collect::<String>();
            let before = text
                .chars()
                .take(*start_codepoint as usize)
                .collect::<String>();
            let after = text
                .chars()
                .skip(*end_codepoint as usize)
                .collect::<String>();
            if &selected != quote || !before.ends_with(prefix) || !after.starts_with(suffix) {
                return Err(Error::InvalidInput(
                    "text locator does not match immutable source",
                ));
            }
        }
        SourceLocator::Pdf {
            page_index,
            crop_box,
            rotation,
            coordinate_space,
            rects,
            ..
        } => {
            if version.format != crate::SourceFormat::Pdf
                || coordinate_space != "pdf_user_space"
                || !matches!(rotation, 0 | 90 | 180 | 270)
                || crop_box.iter().any(|v| !v.is_finite())
                || crop_box[2] <= crop_box[0]
                || crop_box[3] <= crop_box[1]
                || rects.len() > 1000
            {
                return Err(Error::InvalidInput("invalid PDF locator"));
            }
            let page_exists = if history {
                // Schema 9 snapshots predate staging. Later schemas must exclude
                // unfinished parses even if they claim a sufficient page count.
                let has_staging = db.prepare("SELECT 1 FROM sqlite_schema WHERE name='source_parse_staging' AND type='table'")?.exists([])?;
                let sql = if has_staging {
                    "SELECT EXISTS(SELECT 1 FROM source_parses p WHERE p.source_version_id=?1 AND p.page_count>?2 AND NOT EXISTS(SELECT 1 FROM source_parse_staging s WHERE s.parse_id=p.id))"
                } else {
                    "SELECT EXISTS(SELECT 1 FROM source_parses p WHERE p.source_version_id=?1 AND p.page_count>?2)"
                };
                db.query_row(sql, params![version.id, page_index], |row| {
                    row.get::<_, bool>(0)
                })?
            } else {
                let pages:Option<u32>=db.query_row("SELECT p.page_count FROM source_active_parses a JOIN source_parses p ON p.id=a.parse_id WHERE a.source_version_id=?1",[&version.id],|r|r.get(0)).optional()?.flatten();
                pages.is_some_and(|pages| *page_index < pages)
            };
            if !page_exists {
                return Err(Error::InvalidInput("PDF page is unavailable"));
            }
            for rect in rects {
                if rect.iter().any(|v| !v.is_finite())
                    || rect[2] <= rect[0]
                    || rect[3] <= rect[1]
                    || rect[0] < crop_box[0]
                    || rect[1] < crop_box[1]
                    || rect[2] > crop_box[2]
                    || rect[3] > crop_box[3]
                {
                    return Err(Error::InvalidInput("PDF rectangle outside crop box"));
                }
            }
        }
    }
    Ok(())
}

fn reject_staging_locator(db: &Connection, locator: &SourceLocator) -> Result<()> {
    if let SourceLocator::Text { block_id, .. } = locator {
        // Full-package validation also reads immutable pre-schema-10 snapshots.
        let has_staging = db
            .prepare(
                "SELECT 1 FROM sqlite_schema WHERE name='source_parse_staging' AND type='table'",
            )?
            .exists([])?;
        if has_staging && db.prepare("SELECT 1 FROM source_blocks b JOIN source_parse_staging s ON s.parse_id=b.parse_id WHERE b.id=?1")?.exists([block_id])? {
            return Err(Error::NotFound);
        }
    }
    Ok(())
}
pub(crate) fn read_reading(db: &Connection, id: &str) -> Result<ReadingState> {
    let source = crate::sources::read_source(db, id)?;
    // Complete-package verification also reads supported pre-K17 databases.
    let schema: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let view_column = if schema >= 17 {
        "pdf_view_json"
    } else {
        "NULL"
    };
    let row=db.query_row(&format!("SELECT source_version_id,locator_json,status,row_version,{view_column} FROM reading_states WHERE source_id=?1"),[id],|r|Ok((r.get(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get(3)?,r.get::<_,Option<String>>(4)?))).optional()?;
    match row {
        Some((source_version_id, locator, status, row_version, pdf_view)) => {
            let locator: Option<SourceLocator> =
                locator.map(|s| serde_json::from_str(&s)).transpose()?;
            let (parse_id, block_offset) = locator_position(db, locator.as_ref())?;
            Ok(ReadingState {
                source_id: id.into(),
                source_version_id,
                locator,
                parse_id,
                block_offset,
                status: serde_json::from_value(serde_json::Value::String(status))?,
                row_version,
                pdf_view: pdf_view.map(|s| serde_json::from_str(&s)).transpose()?,
            })
        }
        None => Ok(ReadingState {
            source_id: id.into(),
            source_version_id: source.current_version_id,
            locator: None,
            status: ReadingStatus::Unread,
            row_version: 0,
            parse_id: None,
            block_offset: None,
            pdf_view: None,
        }),
    }
}

pub(crate) fn validate_pdf_reading_view(
    db: &Connection,
    version: &crate::SourceVersion,
    locator: Option<&SourceLocator>,
    view: &crate::PdfReadingView,
) -> Result<()> {
    if version.format != crate::SourceFormat::Pdf
        || !view.anchor_x.is_finite()
        || !view.anchor_y.is_finite()
        || !(0.0..=1.0).contains(&view.anchor_x)
        || !(0.0..=1.0).contains(&view.anchor_y)
        || !view.scale.is_finite()
        || !(0.25..=5.0).contains(&view.scale)
        || !matches!(view.rotation, 0 | 90 | 180 | 270)
    {
        return Err(Error::InvalidInput("invalid PDF reading view"));
    }
    if let Some(locator) = locator
        && !matches!(locator, SourceLocator::Pdf { page_index, .. } if *page_index == view.page_index)
    {
        return Err(Error::InvalidInput(
            "PDF reading view and locator page mismatch",
        ));
    }
    let page_count: Option<u32> = db.query_row(
        "SELECT p.page_count FROM source_active_parses a JOIN source_parses p ON p.id=a.parse_id WHERE a.source_version_id=?1",
        [&version.id], |r| r.get(0),
    ).optional()?.flatten();
    if page_count.is_some_and(|count| view.page_index >= count) {
        return Err(Error::InvalidInput("PDF reading page out of bounds"));
    }
    Ok(())
}

pub(crate) fn locator_position(
    db: &Connection,
    locator: Option<&SourceLocator>,
) -> Result<(Option<String>, Option<u32>)> {
    if let Some(locator) = locator {
        reject_staging_locator(db, locator)?;
    }
    let row: Option<(String,u32)> = match locator {
        Some(SourceLocator::Text { source_version_id, block_id, .. }) => db.query_row("SELECT b.parse_id,b.position FROM source_blocks b JOIN source_parses p ON p.id=b.parse_id WHERE b.id=?1 AND p.source_version_id=?2", params![block_id,source_version_id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?,
        Some(SourceLocator::Pdf { source_version_id, page_index, .. }) => db.query_row("SELECT b.parse_id,b.position FROM source_blocks b JOIN source_active_parses a ON a.parse_id=b.parse_id WHERE a.source_version_id=?1 AND b.page_index=?2 ORDER BY b.position LIMIT 1", params![source_version_id,page_index], |r| Ok((r.get(0)?,r.get(1)?))).optional()?,
        None => None,
    };
    Ok(row
        .map(|(id, offset)| (Some(id), Some(offset)))
        .unwrap_or((None, None)))
}

#[derive(Debug)]
pub(crate) struct LocatorSlice {
    pub block_id: String,
    pub parse_id: String,
    pub page_index: Option<u32>,
    pub start_codepoint: u32,
    pub end_codepoint: u32,
    pub text: String,
}

fn pdf_geometry_matches(db: &Connection, locator: &SourceLocator) -> Result<bool> {
    Ok(pdf_evidence_slices(db, locator, None)?.is_some())
}

/// Resolve exact geometry to original block ranges; never widen an approximate locator.
/// An explicit parse is used only to validate an already frozen historical target.
pub(crate) fn pdf_evidence_slices(
    db: &Connection,
    locator: &SourceLocator,
    historical_parse: Option<&str>,
) -> Result<Option<Vec<LocatorSlice>>> {
    let SourceLocator::Pdf {
        source_version_id,
        page_index,
        crop_box,
        rotation,
        coordinate_space,
        rects,
        quote,
        prefix,
        suffix,
        ..
    } = locator
    else {
        return Ok(None);
    };
    if rects.is_empty() {
        return Ok(None);
    }
    let parse: Option<String> = if let Some(parse) = historical_parse {
        db.query_row("SELECT id FROM source_parses p WHERE id=?1 AND source_version_id=?2 AND NOT EXISTS(SELECT 1 FROM source_parse_staging z WHERE z.parse_id=p.id)",params![parse,source_version_id],|r|r.get(0)).optional()?
    } else {
        db.query_row("SELECT a.parse_id FROM source_active_parses a WHERE a.source_version_id=?1 AND NOT EXISTS(SELECT 1 FROM source_parse_staging z WHERE z.parse_id=a.parse_id)",[source_version_id],|r|r.get(0)).optional()?
    };
    let Some(parse) = parse else {
        return Ok(None);
    };
    let blocks=db.prepare("SELECT b.id,b.text,b.page_index,b.geometry_json FROM source_blocks b WHERE b.parse_id=?1 AND b.page_index=?2 ORDER BY b.position")?.query_map(params![parse,page_index],crate::sources::source_block_from_row)?.collect::<std::result::Result<Vec<_>,_>>()?;
    let mut chars = Vec::new();
    let mut covered = Vec::new();
    let mut boundaries = Vec::new();
    for block in blocks {
        let Some(geometry) = block.pdf_geometry else {
            return Ok(None);
        };
        if geometry.rotation != *rotation
            || geometry.coordinate_space != *coordinate_space
            || geometry
                .crop_box
                .iter()
                .zip(crop_box)
                .any(|(a, b)| (a - b).abs() > 0.01)
        {
            return Ok(None);
        }
        let offset = chars.len();
        chars.extend(block.text.chars());
        if geometry.chars.len() != chars.len() - offset {
            return Err(Error::IntegrityFailure);
        }
        for (index, ch) in geometry.chars.into_iter().enumerate() {
            if ch.start_codepoint as usize != index || ch.end_codepoint as usize != index + 1 {
                return Err(Error::IntegrityFailure);
            }
            covered.push(ch.rect.is_some_and(|r| {
                let x = (r[0] + r[2]) * 0.5;
                let y = (r[1] + r[3]) * 0.5;
                rects.iter().any(|s| {
                    x >= s[0] - 0.01 && x <= s[2] + 0.01 && y >= s[1] - 0.01 && y <= s[3] + 0.01
                })
            }));
        }
        boundaries.push((block.id, offset, chars.len()));
    }
    let Some(first) = covered.iter().position(|v| *v) else {
        return Ok(None);
    };
    let last = covered
        .iter()
        .rposition(|v| *v)
        .ok_or(Error::IntegrityFailure)?;
    if (first..=last).any(|i| !covered[i] && !chars[i].is_whitespace()) {
        return Ok(None);
    }
    let normalized = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let selected = chars[first..=last].iter().collect::<String>();
    let before = chars[..first].iter().collect::<String>();
    let after = chars[last + 1..].iter().collect::<String>();
    if normalized(&selected) != normalized(quote)
        || !normalized(&before).ends_with(&normalized(prefix))
        || !normalized(&after).starts_with(&normalized(suffix))
    {
        return Ok(None);
    }
    let slices = boundaries
        .into_iter()
        .filter_map(|(block_id, begin, end)| {
            let from = first.max(begin);
            let to = (last + 1).min(end);
            (from < to).then(|| LocatorSlice {
                block_id,
                parse_id: parse.clone(),
                page_index: Some(*page_index),
                start_codepoint: (from - begin) as u32,
                end_codepoint: (to - begin) as u32,
                text: chars[from..to].iter().collect(),
            })
        })
        .collect();
    Ok(Some(slices))
}

fn finish(
    db: &Connection,
    id: &str,
    hash: &str,
    kind: &str,
    result: &SourceMutation,
) -> Result<()> {
    mutations::store_receipt(db, hash, kind, id, &result.receipt, result)
}
impl Library {
    pub fn annotation(&self, id: &str) -> Result<Annotation> {
        read_annotation(&self.connection, id)
    }
    pub fn create_annotation(
        &mut self,
        command_id: &str,
        request: CreateAnnotation,
    ) -> Result<SourceMutation> {
        if request.comment.len() > 64 * 1024 || request.locator.quote().is_empty() {
            return Err(Error::InvalidInput(
                "annotation requires a bounded nonempty quote",
            ));
        }
        let hash = mutations::request_hash(command_id, "annotations.create", &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = mutations::replay(&tx, command_id, &hash, "annotations.create")? {
            return Ok(old);
        }
        validate_locator(&tx, &request.locator)?;
        let version = crate::sources::read_version(&tx, request.locator.source_version_id())?;
        if crate::sources::read_source(&tx, &version.source_id)?.trashed {
            return Err(Error::Trashed);
        }
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO objects(id,kind,inbox_state) VALUES(?1,'annotation','pending')",
            [&id],
        )?;
        tx.execute("INSERT INTO annotations(id,source_version_id,locator_schema,locator_json,comment) VALUES(?1,?2,1,?3,?4)",params![id,version.id,serde_json::to_string(&request.locator)?,request.comment])?;
        let result = SourceMutation {
            source: None,
            annotation: Some(read_annotation(&tx, &id)?),
            reading: None,
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        finish(&tx, &id, &hash, "annotations.create", &result)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn update_annotation(
        &mut self,
        command_id: &str,
        request: UpdateAnnotation,
    ) -> Result<SourceMutation> {
        if request.comment.len() > 64 * 1024 {
            return Err(Error::InvalidInput("annotation comment too long"));
        }
        let hash = mutations::request_hash(command_id, "annotations.update", &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = mutations::replay(&tx, command_id, &hash, "annotations.update")? {
            return Ok(old);
        }
        let old = read_annotation(&tx, &request.id)?;
        mutations::check_version(&old.id, request.base_row_version, old.row_version)?;
        if old.trashed {
            return Err(Error::Trashed);
        }
        tx.execute(
            "UPDATE annotations SET comment=?2 WHERE id=?1",
            params![old.id, request.comment],
        )?;
        tx.execute(
            "UPDATE objects SET row_version=row_version+1 WHERE id=?1",
            [&old.id],
        )?;
        let result = SourceMutation {
            source: None,
            annotation: Some(read_annotation(&tx, &old.id)?),
            reading: None,
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        finish(&tx, &old.id, &hash, "annotations.update", &result)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn annotations_page(
        &self,
        version_id: &str,
        after_id: Option<&str>,
        limit: u32,
        trashed: bool,
    ) -> Result<Vec<Annotation>> {
        self.source_version(version_id)?;
        crate::collections::validate_limit(limit)?;
        if let Some(id) = after_id {
            valid_uuid(id)?;
        }
        let ids=self.connection.prepare("SELECT a.id FROM annotations a JOIN objects o ON o.id=a.id WHERE source_version_id=?1 AND (?2 IS NULL OR a.id>?2) AND o.trashed=?3 ORDER BY a.id LIMIT ?4")?.query_map(params![version_id,after_id,trashed,limit],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        ids.iter().map(|id| self.annotation(id)).collect()
    }
    pub fn resolve_annotation(&self, id: &str) -> Result<ResolvedAnnotation> {
        let annotation = self.annotation(id)?;
        let version = self.source_version(&annotation.source_version_id)?;
        let source = self.source(&version.source_id)?;
        let (status, reason) = if source.trashed || annotation.trashed {
            (LocatorStatus::Unavailable, Some("target_trashed".into()))
        } else {
            validate_locator(&self.connection, &annotation.locator)?;
            match &annotation.locator {
                SourceLocator::Text { .. } => (LocatorStatus::Exact, None),
                SourceLocator::Pdf { .. } => {
                    if pdf_geometry_matches(&self.connection, &annotation.locator)? {
                        (LocatorStatus::Exact, None)
                    } else {
                        (
                            LocatorStatus::Approximate,
                            Some("page_available_geometry_not_verified".into()),
                        )
                    }
                }
            }
        };
        let (parse_id, block_offset) =
            locator_position(&self.connection, Some(&annotation.locator))?;
        Ok(ResolvedAnnotation {
            annotation,
            status,
            reason,
            parse_id,
            block_offset,
        })
    }
    pub fn reading_state(&self, id: &str) -> Result<ReadingState> {
        read_reading(&self.connection, id)
    }
    pub fn set_reading_state(
        &mut self,
        command_id: &str,
        request: SetReadingState,
    ) -> Result<SourceMutation> {
        let hash = mutations::request_hash(command_id, "reading.set", &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = mutations::replay(&tx, command_id, &hash, "reading.set")? {
            return Ok(old);
        }
        let version = crate::sources::read_version(&tx, &request.source_version_id)?;
        if crate::sources::read_source(&tx, &request.source_id)?.trashed {
            return Err(Error::Trashed);
        }
        if version.source_id != request.source_id {
            return Err(Error::InvalidInput(
                "reading version belongs to another source",
            ));
        }
        let old = read_reading(&tx, &request.source_id)?;
        if request.base_row_version != old.row_version {
            return Err(Error::MetadataConflict {
                object_id: request.source_id,
                expected: request.base_row_version,
                actual: old.row_version,
            });
        }
        if let Some(locator) = &request.locator {
            if locator.source_version_id() != version.id {
                return Err(Error::InvalidInput("reading locator version mismatch"));
            }
            validate_locator(&tx, locator)?;
        }
        if let Some(view) = &request.pdf_view {
            validate_pdf_reading_view(&tx, &version, request.locator.as_ref(), view)?;
        }
        let next = old
            .row_version
            .checked_add(1)
            .ok_or(Error::InvalidInput("reading version overflow"))?;
        tx.execute("INSERT INTO reading_states(source_id,source_version_id,locator_json,status,row_version,pdf_view_json) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(source_id) DO UPDATE SET source_version_id=excluded.source_version_id,locator_json=excluded.locator_json,status=excluded.status,row_version=excluded.row_version,pdf_view_json=excluded.pdf_view_json",params![request.source_id,version.id,request.locator.map(|l|serde_json::to_string(&l)).transpose()?,serde_json::to_value(request.status)?.as_str(),next,request.pdf_view.map(|v|serde_json::to_string(&v)).transpose()?])?;
        let result = SourceMutation {
            source: None,
            annotation: None,
            reading: Some(read_reading(&tx, &request.source_id)?),
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        finish(&tx, &request.source_id, &hash, "reading.set", &result)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn set_source_trashed(
        &mut self,
        command_id: &str,
        request: SetObjectFlag,
    ) -> Result<SourceMutation> {
        self.source_lifecycle(command_id, request, false)
    }
    pub fn set_annotation_trashed(
        &mut self,
        command_id: &str,
        request: SetObjectFlag,
    ) -> Result<SourceMutation> {
        self.source_lifecycle(command_id, request, true)
    }
    fn source_lifecycle(
        &mut self,
        command_id: &str,
        request: SetObjectFlag,
        is_annotation: bool,
    ) -> Result<SourceMutation> {
        let kind = if is_annotation {
            "annotations.trash"
        } else {
            "sources.trash"
        };
        let hash = mutations::request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = mutations::replay(&tx, command_id, &hash, kind)? {
            return Ok(old);
        }
        let version = if is_annotation {
            read_annotation(&tx, &request.object_id)?.row_version
        } else {
            crate::sources::read_source(&tx, &request.object_id)?.row_version
        };
        mutations::check_version(&request.object_id, request.base_row_version, version)?;
        tx.execute(
            "UPDATE objects SET trashed=?2,row_version=row_version+1 WHERE id=?1",
            params![request.object_id, request.value],
        )?;
        let result = SourceMutation {
            source: if is_annotation {
                None
            } else {
                Some(crate::sources::read_source(&tx, &request.object_id)?)
            },
            annotation: if is_annotation {
                Some(read_annotation(&tx, &request.object_id)?)
            } else {
                None
            },
            reading: None,
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        finish(&tx, &request.object_id, &hash, kind, &result)?;
        tx.commit()?;
        Ok(result)
    }
}

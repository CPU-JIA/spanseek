//! Durable review entries point at immutable note/source revisions.
use crate::*;
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

impl mutations::RecordedResult for ReviewMutation {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn canonical_time(s: &str) -> Result<String> {
    Ok(DateTime::parse_from_rfc3339(s)
        .map_err(|_| Error::InvalidInput("invalid review timestamp"))?
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}
fn value<T: Serialize>(v: T) -> Result<String> {
    Ok(serde_json::to_value(v)?
        .as_str()
        .ok_or(Error::IntegrityFailure)?
        .into())
}
fn hash(v: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(v)?)))
}
fn limit(n: u32) -> Result<()> {
    if !(1..=100).contains(&n) {
        return Err(Error::InvalidInput("invalid review page limit"));
    }
    Ok(())
}
fn read(db: &Connection, id: &str) -> Result<ReviewEntry> {
    document::valid_uuid(id)?;
    let row=db.query_row("SELECT target_json,kind,state,due_at,row_version,created_at,updated_at FROM review_entries WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?.ok_or(Error::NotFound)?;
    Ok(ReviewEntry {
        id: id.into(),
        target: serde_json::from_str(&row.0)?,
        kind: serde_json::from_value(serde_json::Value::String(row.1))?,
        state: serde_json::from_value(serde_json::Value::String(row.2))?,
        due_at: row.3,
        row_version: row.4,
        created_at: row.5,
        updated_at: row.6,
    })
}
fn record_event(db: &Connection, e: &ReviewEntry, action: ReviewAction) -> Result<()> {
    db.execute("INSERT INTO review_events(entry_id,seq,action,state,due_at,occurred_at) VALUES(?1,?2,?3,?4,?5,?6)",params![e.id,e.row_version,value(action)?,value(e.state)?,e.due_at,e.updated_at])?;
    Ok(())
}
fn receipt(
    db: &Connection,
    command: &str,
    hash: &str,
    kind: &str,
    id: &str,
) -> Result<ReviewMutation> {
    let r = ReviewMutation {
        entry: read(db, id)?,
        receipt: mutations::next_receipt(db, command)?,
    };
    mutations::store_receipt(db, hash, kind, id, &r.receipt, &r)?;
    Ok(r)
}
fn check_version(e: &ReviewEntry, base: u32) -> Result<()> {
    mutations::check_version(&e.id, base, e.row_version)
}
fn update(
    db: &Connection,
    e: &ReviewEntry,
    state: ReviewState,
    due: &str,
    action: ReviewAction,
) -> Result<()> {
    let occurred_at = now();
    if action == ReviewAction::Snoozed && due <= occurred_at.as_str() {
        return Err(Error::InvalidInput("review due date must be in the future"));
    }
    let version = e
        .row_version
        .checked_add(1)
        .ok_or(Error::InvalidInput("review version overflow"))?;
    db.execute(
        "UPDATE review_entries SET state=?2,due_at=?3,row_version=?4,updated_at=?5 WHERE id=?1",
        params![e.id, value(state)?, due, version, occurred_at],
    )?;
    record_event(db, &read(db, &e.id)?, action)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    scope: String,
    generation: String,
    as_of: String,
    key: String,
    id: String,
    seq: Option<u32>,
}
fn generation(db: &Connection) -> Result<String> {
    let row:(i64,i64,i64,i64,i64)=db.query_row("SELECT next_change_seq,(SELECT generation FROM note_list_state WHERE singleton=1),(SELECT generation FROM source_list_state WHERE singleton=1),(SELECT count(*) FROM review_entries),(SELECT coalesce(sum(row_version),0) FROM review_entries) FROM workspace_meta WHERE singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    hash(&row)
}
impl Library {
    fn review_scope(&self, query: &impl Serialize) -> Result<String> {
        let workspace: String = self.connection.query_row(
            "SELECT workspace_id FROM workspace_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        hash(&(workspace, self.database_path.to_string_lossy(), query))
    }
    pub fn review_entry(&self, id: &str) -> Result<ReviewItem> {
        let _tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        let entry = read(&self.connection, id)?;
        let resolved = self.resolve_link(entry.target.clone())?;
        Ok(ReviewItem { entry, resolved })
    }
    pub fn add_review(&mut self, command: &str, request: AddReview) -> Result<ReviewMutation> {
        let kind = "review.add";
        let digest = mutations::request_hash(command, kind, &request)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &digest, kind)? {
            return Ok(r);
        }
        let resolved = self.resolve_link(request.target.clone())?;
        if !matches!(
            resolved.target_kind,
            Some(ObjectKind::Note | ObjectKind::Source | ObjectKind::Annotation)
        ) {
            return Err(Error::InvalidInput("review target is unavailable"));
        }
        let mut target = request.target;
        target.target_revision_id = Some(
            resolved
                .resolved_revision_id
                .ok_or(Error::InvalidInput("review target has no revision"))?,
        );
        let key = hash(&target)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM review_entries WHERE target_key=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        let id = if let Some(id) = existing {
            let e = read(&tx, &id)?;
            // `completed` and `removed` both mean the entry left the queue, so an
            // explicit add puts it back and needs the caller's current version.
            if matches!(e.state, ReviewState::Completed | ReviewState::Removed) {
                if resolved.state != LinkState::Available {
                    return Err(Error::InvalidInput("review target is unavailable"));
                }
                let requirement = if e.state == ReviewState::Removed {
                    "removed review requires its current version"
                } else {
                    "completed review requires its current version"
                };
                check_version(
                    &e,
                    request
                        .base_row_version
                        .ok_or(Error::InvalidInput(requirement))?,
                )?;
                update(&tx, &e, ReviewState::Pending, &now(), ReviewAction::Added)?;
            }
            id
        } else {
            if resolved.state != LinkState::Available {
                return Err(Error::InvalidInput("review target is unavailable"));
            }
            if request.base_row_version.is_some_and(|v| v != 0) {
                return Err(Error::InvalidInput("new review must omit base version"));
            }
            let id = Uuid::new_v4().to_string();
            let timestamp = now();
            tx.execute("INSERT INTO review_entries(id,target_key,object_id,target_json,kind,state,due_at,row_version,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'pending',?6,1,?6,?6)",params![id,key,target.target_id,serde_json::to_string(&target)?,value(resolved.target_kind.ok_or(Error::IntegrityFailure)?)?,timestamp])?;
            record_event(&tx, &read(&tx, &id)?, ReviewAction::Added)?;
            id
        };
        let r = receipt(&tx, command, &digest, kind, &id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn complete_review(
        &mut self,
        command: &str,
        request: ReviewChange,
    ) -> Result<ReviewMutation> {
        self.change_review(
            command,
            "review.complete",
            request,
            ReviewState::Completed,
            None,
            ReviewAction::Completed,
        )
    }
    pub fn remove_review(
        &mut self,
        command: &str,
        request: ReviewChange,
    ) -> Result<ReviewMutation> {
        self.change_review(
            command,
            "review.remove",
            request,
            ReviewState::Removed,
            None,
            ReviewAction::Removed,
        )
    }
    pub fn snooze_review(
        &mut self,
        command: &str,
        request: SnoozeReview,
    ) -> Result<ReviewMutation> {
        let kind = "review.snooze";
        let digest = mutations::request_hash(command, kind, &request)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &digest, kind)? {
            return Ok(r);
        }
        let due = canonical_time(&request.due_at)?;
        if due <= now() {
            return Err(Error::InvalidInput("review snooze must be in the future"));
        }
        let e = read(&tx, &request.entry_id)?;
        check_version(&e, request.base_row_version)?;
        if e.state == ReviewState::Removed {
            return Err(Error::InvalidInput("removed review must be added again"));
        }
        if self.resolve_link(e.target.clone())?.state != LinkState::Available {
            return Err(Error::InvalidInput("review target is unavailable"));
        }
        update(&tx, &e, ReviewState::Pending, &due, ReviewAction::Snoozed)?;
        let r = receipt(&tx, command, &digest, kind, &e.id)?;
        tx.commit()?;
        Ok(r)
    }
    fn change_review(
        &mut self,
        command: &str,
        kind: &str,
        request: ReviewChange,
        state: ReviewState,
        _due: Option<&str>,
        action: ReviewAction,
    ) -> Result<ReviewMutation> {
        let digest = mutations::request_hash(command, kind, &request)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &digest, kind)? {
            return Ok(r);
        }
        let e = read(&tx, &request.entry_id)?;
        check_version(&e, request.base_row_version)?;
        if state != ReviewState::Removed {
            if e.state == ReviewState::Removed {
                return Err(Error::InvalidInput("removed review must be added again"));
            }
            if self.resolve_link(e.target.clone())?.state != LinkState::Available {
                return Err(Error::InvalidInput("review target is unavailable"));
            }
        }
        if e.state != state {
            update(&tx, &e, state, &e.due_at, action)?;
        }
        let r = receipt(&tx, command, &digest, kind, &e.id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn review_list(&self, request: &ReviewListRequest) -> Result<ReviewPage> {
        limit(request.limit)?;
        if request.cursor.as_ref().is_some_and(|c| c.len() > 4096) {
            return Err(Error::InvalidInput("review cursor exceeds bounds"));
        }
        if let Some(c) = &request.collection_id {
            document::valid_uuid(c)?;
        }
        let _tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        let scope = self.review_scope(&(request.bucket, &request.collection_id, request.limit))?;
        let generation = generation(&self.connection)?;
        let cursor = request
            .cursor
            .as_ref()
            .map(|s| {
                serde_json::from_str::<Cursor>(s)
                    .map_err(|_| Error::InvalidInput("invalid review cursor"))
            })
            .transpose()?;
        if let Some(c) = &cursor {
            if c.scope != scope
                || c.seq.is_some()
                || canonical_time(&c.as_of)? != c.as_of
                || canonical_time(&c.key)? != c.key
                || document::valid_uuid(&c.id).is_err()
            {
                return Err(Error::InvalidInput("review cursor scope mismatch"));
            }
            if c.generation != generation {
                return Err(Error::QueryExpired);
            }
        }
        let as_of = cursor.as_ref().map(|c| c.as_of.clone()).unwrap_or_else(now);
        let due = matches!(request.bucket, ReviewBucket::Due | ReviewBucket::Later);
        let state = match request.bucket {
            ReviewBucket::Due | ReviewBucket::Later => "pending",
            ReviewBucket::Completed => "completed",
            ReviewBucket::Removed => "removed",
        };
        let ordering = if due {
            "e.due_at ASC,e.id ASC"
        } else {
            "e.updated_at DESC,e.id DESC"
        };
        let position = if due {
            "(e.due_at,e.id)>(?5,?6)"
        } else {
            "(e.updated_at,e.id)<(?5,?6)"
        };
        let bucket = value(request.bucket)?;
        let sql = format!(
            "SELECT e.id FROM review_entries e LEFT JOIN annotations annotation ON annotation.id=e.object_id LEFT JOIN source_versions v ON v.id=annotation.source_version_id WHERE e.state=?1 AND (?2!='due' OR e.due_at<=?3) AND (?2!='later' OR e.due_at>?3) AND (?4 IS NULL OR EXISTS(SELECT 1 FROM memberships m JOIN objects c ON c.id=m.collection_id WHERE c.id=?4 AND c.trashed=0 AND (m.member_id=e.object_id OR m.member_id=v.source_id))) AND (?5 IS NULL OR {position}) ORDER BY {ordering} LIMIT ?7"
        );
        let mut ids = self
            .connection
            .prepare(&sql)?
            .query_map(
                params![
                    state,
                    bucket,
                    as_of,
                    request.collection_id,
                    cursor.as_ref().map(|c| &c.key),
                    cursor.as_ref().map(|c| &c.id),
                    request.limit + 1
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = ids.len() > request.limit as usize;
        ids.truncate(request.limit as usize);
        let items = ids
            .iter()
            .map(|id| {
                let entry = read(&self.connection, id)?;
                let resolved = self.resolve_link(entry.target.clone())?;
                Ok(ReviewItem { entry, resolved })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            items
                .last()
                .map(|item| {
                    serde_json::to_string(&Cursor {
                        scope,
                        generation,
                        as_of,
                        key: if due {
                            item.entry.due_at.clone()
                        } else {
                            item.entry.updated_at.clone()
                        },
                        id: item.entry.id.clone(),
                        seq: None,
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(ReviewPage { items, next_cursor })
    }
    /// All four tab counts from one aggregate pass. The 到期/稍后 split uses the
    /// same `due_at<=as_of` / `due_at>as_of` rule as `review_list` and the same
    /// collection filter, so a badge and its list agree at the boundary they
    /// straddle instead of each re-deriving it.
    pub fn review_counts(&self, collection_id: Option<&str>) -> Result<ReviewCounts> {
        if let Some(collection) = collection_id {
            document::valid_uuid(collection)?;
        }
        let as_of = now();
        let (due, later, completed, removed): (i64, i64, i64, i64) = self.connection.query_row(
            "SELECT \
             coalesce(sum(CASE WHEN e.state='pending' AND e.due_at<=?1 THEN 1 ELSE 0 END),0), \
             coalesce(sum(CASE WHEN e.state='pending' AND e.due_at>?1 THEN 1 ELSE 0 END),0), \
             coalesce(sum(CASE WHEN e.state='completed' THEN 1 ELSE 0 END),0), \
             coalesce(sum(CASE WHEN e.state='removed' THEN 1 ELSE 0 END),0) \
             FROM review_entries e \
             LEFT JOIN annotations annotation ON annotation.id=e.object_id \
             LEFT JOIN source_versions v ON v.id=annotation.source_version_id \
             WHERE (?2 IS NULL OR EXISTS(SELECT 1 FROM memberships m JOIN objects c ON c.id=m.collection_id \
                   WHERE c.id=?2 AND c.trashed=0 AND (m.member_id=e.object_id OR m.member_id=v.source_id)))",
            params![as_of, collection_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        Ok(ReviewCounts {
            due: due as u32,
            later: later as u32,
            completed: completed as u32,
            removed: removed as u32,
            as_of,
        })
    }
    pub fn review_history(&self, request: &ReviewHistoryRequest) -> Result<ReviewEventPage> {
        limit(request.limit)?;
        if request.cursor.as_ref().is_some_and(|c| c.len() > 4096) {
            return Err(Error::InvalidInput("review cursor exceeds bounds"));
        }
        let _tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        let e = read(&self.connection, &request.entry_id)?;
        let scope = self.review_scope(&(&request.entry_id, request.limit, "history"))?;
        let cursor = request
            .cursor
            .as_ref()
            .map(|s| {
                serde_json::from_str::<Cursor>(s)
                    .map_err(|_| Error::InvalidInput("invalid review history cursor"))
            })
            .transpose()?;
        if let Some(c) = &cursor {
            if c.scope != scope
                || c.id != e.id
                || c.seq.is_none()
                || c.seq == Some(0)
                || c.seq.is_some_and(|v| v > e.row_version)
            {
                return Err(Error::InvalidInput("review history cursor mismatch"));
            }
            if c.generation != e.row_version.to_string() {
                return Err(Error::QueryExpired);
            }
        }
        let mut events = events(
            &self.connection,
            &e.id,
            cursor.as_ref().and_then(|c| c.seq).unwrap_or(0),
            request.limit + 1,
        )?;
        let more = events.len() > request.limit as usize;
        events.truncate(request.limit as usize);
        let next_cursor = if more {
            events
                .last()
                .map(|event| {
                    serde_json::to_string(&Cursor {
                        scope,
                        generation: e.row_version.to_string(),
                        as_of: String::new(),
                        key: String::new(),
                        id: e.id,
                        seq: Some(event.seq),
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(ReviewEventPage {
            items: events,
            next_cursor,
        })
    }
}
fn events(db: &Connection, id: &str, after: u32, n: u32) -> Result<Vec<ReviewEvent>> {
    let rows=db.prepare("SELECT seq,action,state,due_at,occurred_at FROM review_events WHERE entry_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?.query_map(params![id,after,n],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
    rows.into_iter()
        .map(|(seq, action, state, due_at, occurred_at)| {
            Ok(ReviewEvent {
                entry_id: id.into(),
                seq,
                action: serde_json::from_value(serde_json::Value::String(action))?,
                state: serde_json::from_value(serde_json::Value::String(state))?,
                due_at,
                occurred_at,
            })
        })
        .collect()
}

pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    let ids = db
        .prepare("SELECT id FROM review_entries")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let e = read(db, &id)?;
        let (key,object,kind):(String,String,String)=db.query_row("SELECT r.target_key,r.object_id,o.kind FROM review_entries r JOIN objects o ON o.id=r.object_id WHERE r.id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if key != hash(&e.target)?
            || object != e.target.target_id
            || kind != value(e.kind)?
            || e.row_version == 0
        {
            return Err(Error::IntegrityFailure);
        }
        let revision = e
            .target
            .target_revision_id
            .as_deref()
            .ok_or(Error::IntegrityFailure)?;
        document::valid_uuid(revision)?;
        if let Some(block) = &e.target.target_block_id {
            document::valid_uuid(block)?;
        }
        match e.kind {
            ObjectKind::Note => {
                let note = notes::revision(db, &object, revision)?;
                if e.target
                    .target_block_id
                    .as_ref()
                    .is_some_and(|id| !note.document.nodes().iter().any(|n| n.attrs.id == *id))
                {
                    return Err(Error::IntegrityFailure);
                }
            }
            ObjectKind::Source | ObjectKind::Annotation => {
                let owner = if e.kind == ObjectKind::Annotation {
                    let (version,owner,locator):(String,String,String)=db.query_row("SELECT a.source_version_id,v.source_id,a.locator_json FROM annotations a JOIN source_versions v ON v.id=a.source_version_id WHERE a.id=?1",[&object],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
                    if version != revision {
                        return Err(Error::IntegrityFailure);
                    }
                    if let Some(block) = &e.target.target_block_id {
                        let locator: SourceLocator = serde_json::from_str(&locator)?;
                        if !matches!(locator, SourceLocator::Text { block_id, .. } if block_id == *block)
                        {
                            return Err(Error::IntegrityFailure);
                        }
                    }
                    owner
                } else {
                    object.clone()
                };
                let exists: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM source_versions WHERE id=?1 AND source_id=?2)",
                    params![revision, owner],
                    |r| r.get(0),
                )?;
                if !exists {
                    return Err(Error::IntegrityFailure);
                }
                if let Some(block) = &e.target.target_block_id {
                    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM source_blocks b JOIN source_parses p ON p.id=b.parse_id WHERE b.id=?1 AND p.source_version_id=?2 AND NOT EXISTS(SELECT 1 FROM source_parse_staging s WHERE s.parse_id=p.id))",params![block,revision],|r|r.get(0))?;
                    if !exists {
                        return Err(Error::IntegrityFailure);
                    }
                }
            }
            _ => return Err(Error::IntegrityFailure),
        }
        for date in [&e.due_at, &e.created_at, &e.updated_at] {
            if canonical_time(date)? != *date {
                return Err(Error::IntegrityFailure);
            }
        }
        let all = events(db, &id, 0, u32::MAX)?;
        if all.len() != e.row_version as usize {
            return Err(Error::IntegrityFailure);
        }
        let mut previous = None;
        for (i, event) in all.iter().enumerate() {
            if event.seq as usize != i + 1
                || canonical_time(&event.due_at)? != event.due_at
                || canonical_time(&event.occurred_at)? != event.occurred_at
            {
                return Err(Error::IntegrityFailure);
            }
            let expected = match event.action {
                ReviewAction::Added => {
                    // Re-entering the queue is explicit: an add may follow a removal or
                    // a completed review, and both keep their earlier events.
                    if i != 0
                        && !matches!(
                            previous,
                            Some(ReviewState::Completed | ReviewState::Removed)
                        )
                    {
                        return Err(Error::IntegrityFailure);
                    }
                    ReviewState::Pending
                }
                ReviewAction::Completed => {
                    if previous != Some(ReviewState::Pending) {
                        return Err(Error::IntegrityFailure);
                    }
                    ReviewState::Completed
                }
                ReviewAction::Snoozed => {
                    if !matches!(
                        previous,
                        Some(ReviewState::Pending | ReviewState::Completed)
                    ) || event.due_at <= event.occurred_at
                    {
                        return Err(Error::IntegrityFailure);
                    }
                    ReviewState::Pending
                }
                ReviewAction::Removed => {
                    if !matches!(
                        previous,
                        Some(ReviewState::Pending | ReviewState::Completed)
                    ) {
                        return Err(Error::IntegrityFailure);
                    }
                    ReviewState::Removed
                }
            };
            if event.state != expected {
                return Err(Error::IntegrityFailure);
            }
            previous = Some(event.state);
        }
        let first = all.first().ok_or(Error::IntegrityFailure)?;
        let last = all.last().ok_or(Error::IntegrityFailure)?;
        if first.action != ReviewAction::Added
            || first.occurred_at != e.created_at
            || last.state != e.state
            || last.due_at != e.due_at
            || last.occurred_at != e.updated_at
        {
            return Err(Error::IntegrityFailure);
        }
    }
    Ok(())
}

#[cfg(test)]
mod timing_tests {
    use super::*;
    #[test]
    fn snooze_that_expired_during_resolution_cannot_write_an_invalid_event() {
        let root = tempfile::tempdir().unwrap();
        let mut db = Library::open(root.path()).unwrap();
        let note = db.create_note("review", "body").unwrap();
        let entry = db
            .add_review(
                &Uuid::new_v4().to_string(),
                AddReview {
                    target: ResolveLink {
                        target_id: note.id,
                        target_block_id: None,
                        target_revision_id: None,
                    },
                    base_row_version: None,
                },
            )
            .unwrap()
            .entry;
        let tx = db.connection.transaction().unwrap();
        assert!(
            update(
                &tx,
                &entry,
                ReviewState::Pending,
                "2000-01-01T00:00:00.000Z",
                ReviewAction::Snoozed
            )
            .is_err()
        );
        assert_eq!(read(&tx, &entry.id).unwrap().row_version, 1);
        assert_eq!(events(&tx, &entry.id, 0, 100).unwrap().len(), 1);
    }
}

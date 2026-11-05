//! Persisted rebuild manifests with ordered, explicit outcomes.
use crate::*;
use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;

const MATCHING_INPUTS: &str = "SELECT m.object_id FROM index_rebuild_objects m JOIN index_rebuild_jobs j ON j.id=m.job_id JOIN search_settings settings ON settings.singleton=1 AND settings.active_profile_id=json_extract(j.request_json,'$.profile_id') JOIN objects o ON o.id=m.object_id LEFT JOIN notes n ON n.id=o.id LEFT JOIN note_revisions nr ON nr.revision_id=n.current_revision_id LEFT JOIN sources s ON s.id=o.id LEFT JOIN source_active_parses a ON a.source_version_id=s.current_version_id LEFT JOIN source_parses p ON p.id=a.parse_id WHERE m.job_id=?1 AND m.content_revision=coalesce(n.current_revision_id,s.current_version_id) AND m.metadata_revision=o.row_version AND m.parser_revision=CASE WHEN o.kind='note' THEN 'note-document-'||nr.schema_version ELSE p.parser_version END AND m.parse_id IS CASE WHEN o.kind='note' THEN NULL ELSE a.parse_id END";

fn read(db: &Connection, id: &str) -> Result<RebuildJob> {
    document::valid_uuid(id)?;
    let row=db.query_row("SELECT request_json,status,after_object_id,processed,error_code,created_at,updated_at FROM index_rebuild_jobs WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?.ok_or(Error::NotFound)?;
    Ok(RebuildJob{id:id.into(),request:serde_json::from_str(&row.0)?,status:serde_json::from_value(serde_json::Value::String(row.1))?,after_object_id:row.2,processed:row.3,skipped:db.query_row("SELECT count(*) FROM index_rebuild_objects WHERE job_id=?1 AND skip_reason IS NOT NULL",[id],|r|r.get(0))?,error_code:row.4,created_at:row.5,updated_at:row.6})
}
fn active_profile(db: &Connection, id: &str) -> Result<()> {
    let active: Option<String> = db.query_row(
        "SELECT active_profile_id FROM search_settings WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    if active.as_deref() != Some(id) {
        return Err(Error::InvalidInput("rebuild embedding profile changed"));
    }
    Ok(())
}
fn check(db: &Connection, id: &str, token: &str) -> Result<RebuildJob> {
    let job = check_lease(db, id, token)?;
    active_profile(db, &job.request.profile_id)?;
    Ok(job)
}
fn check_lease(db: &Connection, id: &str, token: &str) -> Result<RebuildJob> {
    let job = read(db, id)?;
    let valid:bool=db.query_row("SELECT status='running' AND lease_token=?2 AND lease_expires_at>?3 FROM index_rebuild_jobs WHERE id=?1",params![id,token,Utc::now().to_rfc3339()],|r|r.get(0))?;
    if !valid {
        return Err(Error::InvalidInput("rebuild lease inactive"));
    }
    Ok(job)
}
fn limit(n: u32) -> Result<()> {
    if !(1..=100).contains(&n) {
        return Err(Error::InvalidInput("invalid rebuild page limit"));
    }
    Ok(())
}
fn code(s: &str) -> Result<()> {
    if s.is_empty()
        || s.len() > 80
        || !s
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(Error::InvalidInput("invalid rebuild reason"));
    }
    Ok(())
}
fn next(db: &Connection, id: &str, n: u32) -> Result<Vec<String>> {
    limit(n)?;
    Ok(db.prepare("SELECT object_id FROM index_rebuild_objects WHERE job_id=?1 AND object_id>coalesce((SELECT after_object_id FROM index_rebuild_jobs WHERE id=?1),'') AND generation_id IS NULL AND skip_reason IS NULL ORDER BY object_id LIMIT ?2")?.query_map(params![id,n],|r|r.get(0))?.collect::<std::result::Result<Vec<_>,_>>()?)
}
fn eligible(db: &Connection, scope: &SearchScope) -> Result<Vec<String>> {
    let scoped = retrieval::eligible_ids(db, scope)?;
    Ok(db.prepare("SELECT o.id FROM objects o WHERE o.id IN (SELECT value FROM json_each(?1)) AND (o.kind='note' OR EXISTS(SELECT 1 FROM sources s JOIN source_active_parses a ON a.source_version_id=s.current_version_id WHERE s.id=o.id)) ORDER BY o.id")?.query_map([serde_json::to_string(&scoped)?],|r|r.get(0))?.collect::<std::result::Result<Vec<_>,_>>()?)
}
fn generation_current(
    db: &Connection,
    job: &RebuildJob,
    object: &str,
    generation: &str,
) -> Result<bool> {
    let g = crate::index::generation(db, generation)?;
    if g.status != IndexGenerationStatus::Ready
        || g.object_id != object
        || g.profile_id != job.request.profile_id
    {
        return Ok(false);
    }
    let current:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM active_index_generations a JOIN objects o ON o.id=a.object_id LEFT JOIN notes n ON n.id=o.id LEFT JOIN note_revisions r ON r.revision_id=n.current_revision_id LEFT JOIN sources s ON s.id=o.id LEFT JOIN source_active_parses p ON p.source_version_id=s.current_version_id LEFT JOIN source_parses sp ON sp.id=p.parse_id WHERE a.generation_id=?1 AND a.profile_id=?2 AND o.row_version=?3 AND coalesce(n.current_revision_id,s.current_version_id)=?4 AND CASE WHEN o.kind='note' THEN 'note-document-'||r.schema_version ELSE sp.parser_version END=?5)",params![generation,job.request.profile_id,g.metadata_revision,g.content_revision,g.parser_revision],|r|r.get(0))?;
    Ok(current)
}
fn finish(db: &Connection, id: &str, status: &str, error: Option<&str>) -> Result<()> {
    db.execute("UPDATE index_rebuild_jobs SET status=?2,error_code=?3,lease_token=NULL,lease_expires_at=NULL,updated_at=?4 WHERE id=?1",params![id,status,error,Utc::now().to_rfc3339()])?;
    Ok(())
}
fn receipt(
    db: &Connection,
    command: &str,
    hash: &str,
    kind: &str,
    id: &str,
) -> Result<RebuildMutation> {
    let r = RebuildMutation {
        job: read(db, id)?,
        receipt: mutations::next_receipt(db, command)?,
    };
    mutations::store_receipt(db, hash, kind, id, &r.receipt, &r)?;
    Ok(r)
}
impl Library {
    /// Publish a generation and advance its rebuild atomically against cancellation.
    pub fn publish_rebuild_generation(
        &mut self,
        id: &str,
        token: &str,
        publish: IndexPublish,
    ) -> Result<RebuildJob> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = check(&tx, id, token)?;
        let generation = crate::index::generation(&tx, &publish.generation_id)?;
        let object = &generation.object_id;
        let prior:Option<(Option<String>,Option<String>)>=tx.query_row("SELECT generation_id,skip_reason FROM index_rebuild_objects WHERE job_id=?1 AND object_id=?2",params![id,object],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((old, skip)) = prior else {
            return Err(Error::InvalidInput(
                "generation object outside rebuild manifest",
            ));
        };
        if old.is_some() || skip.is_some() {
            if old.as_deref() == Some(publish.generation_id.as_str()) && skip.is_none() {
                crate::index::publish(&tx, &publish)?;
                return Ok(job);
            }
            return Err(Error::IdempotencyMismatch);
        }
        if next(&tx, id, 1)?.first() != Some(object) {
            return Err(Error::InvalidInput("rebuild publication is out of order"));
        }
        let unchanged: bool = tx.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM ({MATCHING_INPUTS}) WHERE object_id=?2)"),
            params![id, object],
            |r| r.get(0),
        )?;
        if !unchanged {
            return Err(Error::InvalidInput("rebuild frozen input changed"));
        }
        let mut scope = job.request.scope.clone();
        if !scope.object_ids.is_empty() && !scope.object_ids.contains(object) {
            return Err(Error::InvalidInput(
                "generation object outside rebuild scope",
            ));
        }
        scope.object_ids = vec![object.clone()];
        if eligible(&tx, &scope)?.is_empty() {
            return Err(Error::InvalidInput("rebuild object no longer eligible"));
        }
        let published = crate::index::publish(&tx, &publish)?;
        if !generation_current(&tx, &job, object, &published.id)? {
            return Err(Error::InvalidInput("rebuild generation is not current"));
        }
        tx.execute(
            "UPDATE index_rebuild_objects SET generation_id=?3 WHERE job_id=?1 AND object_id=?2",
            params![id, object, published.id],
        )?;
        tx.execute("UPDATE index_rebuild_jobs SET after_object_id=?2,processed=processed+1,updated_at=?3 WHERE id=?1",params![id,object,Utc::now().to_rfc3339()])?;
        let result = read(&tx, id)?;
        tx.commit()?;
        Ok(result)
    }
    /// Caller must hold this workspace's exclusive WorkspaceLease and stop all old workers.
    pub fn recover_index_rebuilds_exclusive(&mut self) -> Result<u32> {
        Ok(self.connection.execute("UPDATE index_rebuild_jobs SET status='failed',error_code='interrupted',lease_token=NULL,lease_expires_at=NULL,updated_at=?1 WHERE status='running'",[Utc::now().to_rfc3339()])? as u32)
    }
    pub fn accept_index_rebuild(
        &mut self,
        command: &str,
        request: RebuildRequest,
    ) -> Result<RebuildMutation> {
        let kind = "search.rebuild";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        active_profile(&tx, &request.profile_id)?;
        let objects = eligible(&tx, &request.scope)?;
        if objects.len() > 100_000 {
            return Err(Error::InvalidInput("rebuild scope exceeds 100000 objects"));
        }
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO index_rebuild_jobs(id,request_json,status,created_at,updated_at) VALUES(?1,?2,'queued',?3,?3)",params![id,serde_json::to_string(&request)?,now])?;
        tx.execute("INSERT INTO index_rebuild_objects(job_id,object_id,content_revision,metadata_revision,parser_revision,parse_id) SELECT ?1,o.id,coalesce(n.current_revision_id,s.current_version_id),o.row_version,CASE WHEN o.kind='note' THEN 'note-document-'||nr.schema_version ELSE p.parser_version END,CASE WHEN o.kind='note' THEN NULL ELSE a.parse_id END FROM json_each(?2) e JOIN objects o ON o.id=e.value LEFT JOIN notes n ON n.id=o.id LEFT JOIN note_revisions nr ON nr.revision_id=n.current_revision_id LEFT JOIN sources s ON s.id=o.id LEFT JOIN source_active_parses a ON a.source_version_id=s.current_version_id LEFT JOIN source_parses p ON p.id=a.parse_id",params![id,serde_json::to_string(&objects)?])?;
        let result = receipt(&tx, command, &hash, kind, &id)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn index_rebuild_job(&self, id: &str) -> Result<RebuildJob> {
        read(&self.connection, id)
    }
    pub fn pending_index_rebuild_ids(&self, n: u32) -> Result<Vec<String>> {
        limit(n)?;
        Ok(self.connection.prepare("SELECT id FROM index_rebuild_jobs WHERE status='queued' ORDER BY created_at,id LIMIT ?1")?.query_map([n],|r|r.get(0))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }
    pub fn index_rebuild_jobs_page(&self, after: Option<&str>, n: u32) -> Result<Vec<RebuildJob>> {
        limit(n)?;
        let cursor = after.map(|id| read(&self.connection, id)).transpose()?;
        let ids=self.connection.prepare("SELECT id FROM index_rebuild_jobs WHERE ?1 IS NULL OR (created_at,id)<(?1,?2) ORDER BY created_at DESC,id DESC LIMIT ?3")?.query_map(params![cursor.as_ref().map(|j|&j.created_at),after,n],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        ids.iter().map(|id| read(&self.connection, id)).collect()
    }
    pub fn claim_index_rebuild(&mut self, id: &str) -> Result<Option<RebuildLease>> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = read(&tx, id)?;
        if job.status != RebuildStatus::Queued {
            return Ok(None);
        }
        match active_profile(&tx, &job.request.profile_id) {
            Ok(()) => {}
            Err(Error::InvalidInput(_)) => {
                finish(&tx, id, "failed", Some("profile_changed"))?;
                tx.commit()?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        let token = Uuid::new_v4().to_string();
        tx.execute("UPDATE index_rebuild_jobs SET status='running',lease_token=?2,lease_expires_at=?3,updated_at=?4 WHERE id=?1",params![id,token,(Utc::now()+Duration::minutes(5)).to_rfc3339(),Utc::now().to_rfc3339()])?;
        let job = read(&tx, id)?;
        tx.commit()?;
        Ok(Some(RebuildLease {
            job,
            lease_token: token,
        }))
    }
    pub fn heartbeat_index_rebuild(&mut self, id: &str, token: &str) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check(&tx, id, token)?;
        tx.execute(
            "UPDATE index_rebuild_jobs SET lease_expires_at=?2,updated_at=?3 WHERE id=?1",
            params![
                id,
                (Utc::now() + Duration::minutes(5)).to_rfc3339(),
                Utc::now().to_rfc3339()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn index_rebuild_next_objects(&self, id: &str, n: u32) -> Result<Vec<String>> {
        read(&self.connection, id)?;
        next(&self.connection, id, n)
    }
    pub fn checkpoint_index_rebuild(
        &mut self,
        id: &str,
        token: &str,
        object: &str,
        generation: Option<&str>,
        skip: Option<&str>,
    ) -> Result<RebuildJob> {
        document::valid_uuid(object)?;
        if generation.is_some() == skip.is_some() {
            return Err(Error::InvalidInput(
                "checkpoint needs generation or explicit skip",
            ));
        }
        if let Some(s) = skip {
            code(s)?;
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = check(&tx, id, token)?;
        let prior:Option<(Option<String>,Option<String>)>=tx.query_row("SELECT generation_id,skip_reason FROM index_rebuild_objects WHERE job_id=?1 AND object_id=?2",params![id,object],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((old_generation, old_skip)) = prior else {
            return Err(Error::InvalidInput("object not in rebuild manifest"));
        };
        if old_generation.is_some() || old_skip.is_some() {
            if old_generation.as_deref() == generation && old_skip.as_deref() == skip {
                return Ok(job);
            }
            return Err(Error::IdempotencyMismatch);
        }
        if next(&tx, id, 1)?.first().map(String::as_str) != Some(object) {
            return Err(Error::InvalidInput("rebuild checkpoint is out of order"));
        }
        if let Some(g) = generation {
            let mut scope = job.request.scope.clone();
            if !scope.object_ids.is_empty() && !scope.object_ids.iter().any(|id| id == object) {
                return Err(Error::InvalidInput("rebuild object outside scope"));
            }
            scope.object_ids = vec![object.into()];
            if eligible(&tx, &scope)?.is_empty() || !generation_current(&tx, &job, object, g)? {
                return Err(Error::InvalidInput("rebuild generation is not current"));
            }
        }
        tx.execute("UPDATE index_rebuild_objects SET generation_id=?3,skip_reason=?4 WHERE job_id=?1 AND object_id=?2",params![id,object,generation,skip])?;
        tx.execute("UPDATE index_rebuild_jobs SET after_object_id=?2,processed=processed+1,updated_at=?3 WHERE id=?1",params![id,object,Utc::now().to_rfc3339()])?;
        let r = read(&tx, id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn finish_index_rebuild(&mut self, id: &str, token: &str) -> Result<RebuildJob> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = check(&tx, id, token)?;
        if !next(&tx, id, 1)?.is_empty() {
            return Err(Error::InvalidInput("rebuild still has pending objects"));
        }
        let objects = eligible(&tx, &job.request.scope)?;
        let changed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM json_each(?1) e LEFT JOIN index_rebuild_objects m ON m.job_id=?2 AND m.object_id=e.value LEFT JOIN index_generations g ON g.id=m.generation_id LEFT JOIN active_index_generations a ON a.object_id=e.value AND a.profile_id=?3 JOIN objects o ON o.id=e.value LEFT JOIN notes n ON n.id=o.id LEFT JOIN note_revisions nr ON nr.revision_id=n.current_revision_id LEFT JOIN sources s ON s.id=o.id LEFT JOIN source_active_parses p ON p.source_version_id=s.current_version_id LEFT JOIN source_parses sp ON sp.id=p.parse_id WHERE m.skip_reason IS NULL AND (g.id IS NULL OR g.status!='ready' OR g.object_id!=e.value OR g.profile_id!=?3 OR a.generation_id IS NOT g.id OR g.metadata_revision!=o.row_version OR g.content_revision IS NOT coalesce(n.current_revision_id,s.current_version_id) OR g.parser_revision IS NOT CASE WHEN o.kind='note' THEN 'note-document-'||nr.schema_version ELSE sp.parser_version END))",params![serde_json::to_string(&objects)?,id,job.request.profile_id],|r|r.get(0))?;
        finish(
            &tx,
            id,
            if changed { "failed" } else { "completed" },
            changed.then_some("scope_changed"),
        )?;
        let r = read(&tx, id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn fail_index_rebuild(&mut self, id: &str, token: &str, error: &str) -> Result<RebuildJob> {
        code(error)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_lease(&tx, id, token)?;
        finish(&tx, id, "failed", Some(error))?;
        let r = read(&tx, id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn cancel_index_rebuild(&mut self, command: &str, id: &str) -> Result<RebuildMutation> {
        let kind = "search.rebuild_cancel";
        let hash = mutations::request_hash(command, kind, &id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        if matches!(
            read(&tx, id)?.status,
            RebuildStatus::Queued | RebuildStatus::Running
        ) {
            tx.execute(
                &format!("DELETE FROM index_dirty_objects WHERE object_id IN ({MATCHING_INPUTS})"),
                [id],
            )?;
            finish(&tx, id, "canceled", Some("canceled"))?;
        }
        let r = receipt(&tx, command, &hash, kind, id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn recover_expired_index_rebuilds(&mut self) -> Result<u32> {
        Ok(self.connection.execute("UPDATE index_rebuild_jobs SET status='failed',error_code='interrupted',lease_token=NULL,lease_expires_at=NULL,updated_at=?1 WHERE status='running' AND lease_expires_at<=?1",[Utc::now().to_rfc3339()])? as u32)
    }
    pub fn defer_dirty_index_rebuild(&mut self, id: &str, error: &str) -> Result<()> {
        code(error)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        read(&tx, id)?;
        let Some(failed_object) = next(&tx, id, 1)?.into_iter().next() else {
            return Ok(());
        };
        tx.execute(&format!("UPDATE index_dirty_objects SET attempts=CASE WHEN ?4 THEN 3 ELSE min(attempts+1,3) END,next_retry_at=strftime('%Y-%m-%dT%H:%M:%f+00:00',?2,'+'||min(30*(1<<min(attempts,7)),3600)||' seconds'),last_error=?3 WHERE object_id=?5 AND object_id IN ({MATCHING_INPUTS})"),params![id,Utc::now().to_rfc3339(),error,crate::index::deterministic_index_error(error),failed_object])?;
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    let invalid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM index_rebuild_objects m JOIN objects o ON o.id=m.object_id LEFT JOIN note_revisions n ON n.revision_id=m.content_revision AND n.note_id=m.object_id LEFT JOIN source_versions v ON v.id=m.content_revision AND v.source_id=m.object_id LEFT JOIN source_parses p ON p.id=m.parse_id AND p.source_version_id=v.id WHERE m.metadata_revision<1 OR (o.kind='note' AND (n.revision_id IS NULL OR m.parse_id IS NOT NULL OR m.parser_revision!='note-document-'||n.schema_version)) OR (o.kind='source' AND (v.id IS NULL OR p.id IS NULL OR m.parser_revision!=p.parser_version)) OR o.kind NOT IN ('note','source'))",[],|r|r.get(0))?;
    if invalid {
        return Err(Error::IntegrityFailure);
    }
    let ids = db
        .prepare("SELECT id FROM index_rebuild_jobs")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let job = read(db, &id)?;
        chrono::DateTime::parse_from_rfc3339(&job.created_at)
            .map_err(|_| Error::IntegrityFailure)?;
        chrono::DateTime::parse_from_rfc3339(&job.updated_at)
            .map_err(|_| Error::IntegrityFailure)?;
        retrieval::eligible_ids(db, &job.request.scope)?;
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM embedding_profiles WHERE id=?1)",
            [&job.request.profile_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::IntegrityFailure);
        }
        if let Some(s) = &job.error_code {
            code(s)?;
        }
        let (token, expires): (Option<String>, Option<String>) = db.query_row(
            "SELECT lease_token,lease_expires_at FROM index_rebuild_jobs WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if job.status == RebuildStatus::Running {
            document::valid_uuid(token.as_deref().ok_or(Error::IntegrityFailure)?)?;
            chrono::DateTime::parse_from_rfc3339(
                expires.as_deref().ok_or(Error::IntegrityFailure)?,
            )
            .map_err(|_| Error::IntegrityFailure)?;
        } else if token.is_some() || expires.is_some() {
            return Err(Error::IntegrityFailure);
        }
        let rows=db.prepare("SELECT object_id,generation_id,skip_reason FROM index_rebuild_objects WHERE job_id=?1 ORDER BY object_id")?.query_map([&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut done = BTreeSet::new();
        let mut pending = false;
        for (object, generation, skip) in rows {
            document::valid_uuid(&object)?;
            if generation.is_some() && skip.is_some() {
                return Err(Error::IntegrityFailure);
            }
            if generation.is_none() && skip.is_none() {
                pending = true;
                continue;
            }
            if pending {
                return Err(Error::IntegrityFailure);
            }
            if let Some(g) = generation {
                let g = crate::index::generation(db, &g)?;
                if g.object_id != object || g.profile_id != job.request.profile_id {
                    return Err(Error::IntegrityFailure);
                }
            }
            if let Some(s) = skip {
                code(&s)?;
            }
            done.insert(object);
        }
        if done.len() != job.processed as usize
            || done.last() != job.after_object_id.as_ref()
            || (job.status == RebuildStatus::Completed && pending)
            || (job.status == RebuildStatus::Queued && job.processed != 0)
        {
            return Err(Error::IntegrityFailure);
        }
    }
    Ok(())
}

use crate::*;
use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

// Shared by enumeration and the availability check so both validate the same
// active profile, object lifecycle, revisions and parser version in one snapshot.
const ACTIVE_GENERATION_RELATIONS: &str = "FROM active_index_generations a
             JOIN search_settings ss ON ss.active_profile_id=a.profile_id
             JOIN index_generations g ON g.id=a.generation_id
                 AND g.object_id=a.object_id AND g.profile_id=a.profile_id
             JOIN objects o ON o.id=g.object_id
             LEFT JOIN notes n ON n.id=o.id
             LEFT JOIN note_revisions r ON r.revision_id=n.current_revision_id
             LEFT JOIN sources s ON s.id=o.id
             LEFT JOIN source_active_parses ap ON ap.source_version_id=s.current_version_id
             LEFT JOIN source_parses p ON p.id=ap.parse_id
             WHERE g.status='ready' AND o.trashed=0 AND o.search_enabled=1
                 AND o.row_version=g.metadata_revision
                 AND coalesce(n.current_revision_id,s.current_version_id)=g.content_revision
                 AND CASE WHEN o.kind='note' THEN 'note-document-'||r.schema_version
                          ELSE p.parser_version END=g.parser_revision";

fn evidence_fusion_key(evidence: &SearchEvidence) -> Result<String> {
    // Profile is a scoring property, not part of the annotation authorization.
    Ok(serde_json::to_string(&(
        &evidence.target,
        evidence
            .annotation
            .as_ref()
            .map(|a| (&a.annotation_id, a.annotation_revision, &a.locator_hash)),
    ))?)
}

pub(crate) fn deterministic_index_error(code: &str) -> bool {
    matches!(
        code,
        "embedding_profile_mismatch"
            | "embedding_model_invalid"
            | "embedding_model_hash_mismatch"
            | "embedding_model_load_failed"
            | "embedding_tokenizer_invalid"
            | "embedding_tokenization_failed"
            | "embedding_title_token_limit"
            | "embedding_input_limit"
            | "embedding_token_budget"
            | "embedding_chunk_size_limit"
            | "embedding_chunk_limit"
            | "embedding_batch_limit"
            | "embedding_invalid_input"
            | "embedding_invalid_plan"
            | "embedding_invalid_vector"
            | "embedding_invalid_vector_count"
            | "invalid_input"
    )
}

/// Plain HTTP is only acceptable for an embedding server on this machine.
fn loopback_host(rest: &str) -> bool {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}
fn valid_embedding_endpoint(endpoint: &str) -> bool {
    if endpoint.len() > 2048
        || endpoint.trim() != endpoint
        || endpoint.chars().any(char::is_control)
    {
        return false;
    }
    let rest = if let Some(rest) = endpoint.strip_prefix("https://") {
        rest
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        if !loopback_host(rest) {
            return false;
        }
        rest
    } else {
        return false;
    };
    !rest.is_empty() && !rest.starts_with('/')
}
fn validate_profile_spec(spec: &EmbeddingProfileSpec) -> Result<()> {
    if let Some(endpoint) = &spec.endpoint
        && !valid_embedding_endpoint(endpoint)
    {
        return Err(Error::InvalidInput("invalid embedding endpoint"));
    }
    if spec
        .output_dimensions
        .is_some_and(|d| !(1..=4096).contains(&d) || d != spec.dimensions)
    {
        return Err(Error::InvalidInput("invalid embedding dimensions"));
    }
    for text in [&spec.model, &spec.revision, &spec.tokenizer, &spec.chunker] {
        if text.trim().is_empty() || text.len() > 1024 || text.chars().any(char::is_control) {
            return Err(Error::InvalidInput("invalid embedding profile"));
        }
    }
    if !(1..=4096).contains(&spec.dimensions)
        || !(1..=32768).contains(&spec.max_tokens)
        || spec.query_prefix.len() > 1024
        || spec.passage_prefix.len() > 1024
    {
        return Err(Error::InvalidInput(
            "invalid embedding dimensions or token budget",
        ));
    }
    Ok(())
}

pub(crate) fn profile(db: &Connection, id: &str) -> Result<EmbeddingProfile> {
    let json: String = db
        .query_row(
            "SELECT spec_json FROM embedding_profiles WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Ok(EmbeddingProfile {
        id: id.into(),
        spec: serde_json::from_str(&json)?,
    })
}
fn input_matches(
    db: &Connection,
    object: &str,
    revision: &str,
    metadata: u32,
    parser: &str,
) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM objects o LEFT JOIN notes n ON n.id=o.id LEFT JOIN note_revisions r ON r.revision_id=n.current_revision_id LEFT JOIN sources s ON s.id=o.id LEFT JOIN source_active_parses a ON a.source_version_id=s.current_version_id LEFT JOIN source_parses p ON p.id=a.parse_id WHERE o.id=?1 AND o.trashed=0 AND o.search_enabled=1 AND o.row_version=?2 AND coalesce(n.current_revision_id,s.current_version_id)=?3 AND CASE WHEN o.kind='note' THEN 'note-document-'||r.schema_version ELSE p.parser_version END=?4)",params![object,metadata,revision,parser],|r|r.get(0))?)
}
pub(crate) fn generation(db: &Connection, id: &str) -> Result<IndexGeneration> {
    db.query_row("SELECT object_id,content_revision,metadata_revision,profile_id,parser_revision,status,chunk_count,error_code,created_at FROM index_generations WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get::<_,String>(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?.map(|row|Ok(IndexGeneration{id:id.into(),object_id:row.0,content_revision:row.1,metadata_revision:row.2,profile_id:row.3,parser_revision:row.4,status:serde_json::from_value(serde_json::Value::String(row.5))?,chunk_count:row.6,error_code:row.7,created_at:row.8})).unwrap_or(Err(Error::NotFound))
}
fn check_lease(db: &Connection, id: &str, lease: &str) -> Result<IndexGeneration> {
    let g = generation(db, id)?;
    let valid:bool=db.query_row("SELECT status='running' AND lease_token=?2 AND lease_expires_at>?3 FROM index_generations WHERE id=?1",params![id,lease,Utc::now().to_rfc3339()],|r|r.get(0))?;
    if !valid {
        return Err(Error::InvalidInput("index lease expired or canceled"));
    }
    let active: Option<String> = db.query_row(
        "SELECT active_profile_id FROM search_settings WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    if active.as_deref() != Some(&g.profile_id) {
        return Err(Error::InvalidInput("embedding profile changed"));
    }
    let identity:Option<(String,u32,String)>=db.query_row("SELECT coalesce(n.current_revision_id,s.current_version_id),o.row_version,CASE WHEN o.kind='note' THEN 'note-document-'||r.schema_version ELSE p.parser_version END FROM objects o LEFT JOIN notes n ON n.id=o.id LEFT JOIN note_revisions r ON r.revision_id=n.current_revision_id LEFT JOIN sources s ON s.id=o.id LEFT JOIN source_active_parses a ON a.source_version_id=s.current_version_id LEFT JOIN source_parses p ON p.id=a.parse_id WHERE o.id=?1 AND o.trashed=0 AND o.search_enabled=1 AND (n.id IS NOT NULL OR a.parse_id IS NOT NULL)",[&g.object_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if !identity.is_some_and(|(revision, metadata, parser)| {
        revision == g.content_revision
            && metadata == g.metadata_revision
            && parser == g.parser_revision
    }) {
        return Err(Error::InvalidInput("index input changed"));
    }
    let eligible = crate::retrieval::eligible_ids(
        db,
        &SearchScope {
            object_ids: vec![g.object_id.clone()],
            ..Default::default()
        },
    )?;
    if eligible.is_empty() {
        return Err(Error::InvalidInput("index input unavailable"));
    }
    Ok(g)
}
pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    let dirty = db
        .prepare("SELECT object_id,attempts,next_retry_at,last_error FROM index_dirty_objects")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, attempts, retry, error) in dirty {
        crate::document::valid_uuid(&id)?;
        if attempts > 3
            || retry
                .as_ref()
                .is_some_and(|v| chrono::DateTime::parse_from_rfc3339(v).is_err())
            || error.as_ref().is_some_and(|v| {
                v.is_empty()
                    || v.len() > 100
                    || !v
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
            || (attempts == 0 && (retry.is_some() || error.is_some()))
            || (attempts > 0 && (retry.is_none() || error.is_none()))
        {
            return Err(Error::IntegrityFailure);
        }
    }
    let profiles = db
        .prepare("SELECT id,spec_json FROM embedding_profiles")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, json) in profiles {
        let spec: EmbeddingProfileSpec = serde_json::from_str(&json)?;
        validate_profile_spec(&spec)?;
        if id != format!("{:x}", Sha256::digest(serde_json::to_vec(&spec)?))
            || spec.model.is_empty()
            || spec.revision.is_empty()
            || spec.tokenizer.is_empty()
            || spec.chunker.is_empty()
            || !(1..=4096).contains(&spec.dimensions)
            || !(1..=32768).contains(&spec.max_tokens)
        {
            return Err(Error::IntegrityFailure);
        }
    }
    let identities=db.prepare("SELECT id,content_revision,parser_revision,created_at,lease_token,lease_expires_at,request_hash FROM index_generations")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
    for (id, revision, parser, created, lease, expires, hash) in identities {
        crate::document::valid_uuid(&id)?;
        crate::document::valid_uuid(&revision)?;
        if parser.is_empty()
            || parser.len() > 1024
            || parser.chars().any(char::is_control)
            || chrono::DateTime::parse_from_rfc3339(&created).is_err()
            || hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::IntegrityFailure);
        }
        if let Some(lease) = lease {
            crate::document::valid_uuid(&lease)?;
        }
        if expires.is_some_and(|value| chrono::DateTime::parse_from_rfc3339(&value).is_err()) {
            return Err(Error::IntegrityFailure);
        }
    }
    for sql in [
        "SELECT 1 FROM active_index_generations a JOIN index_generations g ON g.id=a.generation_id WHERE a.object_id!=g.object_id OR a.profile_id!=g.profile_id OR g.status!='ready'",
        "SELECT 1 FROM index_generations g WHERE chunk_count<1 OR chunk_count>100000 OR metadata_revision<1 OR (status='running' AND (lease_token IS NULL OR lease_expires_at IS NULL)) OR (SELECT count(*) FROM index_chunks c WHERE c.generation_id=g.id)>chunk_count OR (status='ready' AND (SELECT count(*) FROM index_chunks c WHERE c.generation_id=g.id)!=chunk_count)",
        "SELECT 1 FROM index_chunks GROUP BY generation_id HAVING sum(length(CAST(text AS BLOB)))>67108864",
    ] {
        if db.prepare(sql)?.exists([])? {
            return Err(Error::IntegrityFailure);
        }
    }
    // Retain one revision and one block's character table, not one decoded revision
    // per chunk or all source blocks. The outer cursor also keeps one read snapshot.
    let mut generations = db.prepare("SELECT g.id,p.spec_json,o.kind FROM index_generations g JOIN embedding_profiles p ON p.id=g.profile_id JOIN objects o ON o.id=g.object_id WHERE EXISTS(SELECT 1 FROM index_chunks c WHERE c.generation_id=g.id) ORDER BY g.id")?;
    let mut generation_rows = generations.query([])?;
    let mut chunks = db.prepare("SELECT id,target_json,title,text,token_count FROM index_chunks WHERE generation_id=?1 ORDER BY json_extract(target_json,'$.block_id'),id")?;
    let mut source_block = db.prepare("SELECT b.text,b.page_index,p.parser_version FROM source_blocks b JOIN source_parses p ON p.id=b.parse_id JOIN source_versions v ON v.id=p.source_version_id WHERE b.id=?1 AND b.parse_id=?2 AND v.id=?3 AND v.source_id=?4 AND NOT EXISTS(SELECT 1 FROM source_parse_staging z WHERE z.parse_id=p.id)")?;
    while let Some(generation_row) = generation_rows.next()? {
        let g = generation(db, &generation_row.get::<_, String>(0)?)?;
        let profile: EmbeddingProfileSpec =
            serde_json::from_str(&generation_row.get::<_, String>(1)?)?;
        let kind = match generation_row.get::<_, String>(2)?.as_str() {
            "note" => EvidenceKind::Note,
            "source" => EvidenceKind::Source,
            _ => return Err(Error::IntegrityFailure),
        };
        let note = if kind == EvidenceKind::Note {
            let note = crate::notes::revision(db, &g.object_id, &g.content_revision)?;
            if g.parser_revision != format!("note-document-{}", note.schema_version) {
                return Err(Error::IntegrityFailure);
            }
            Some(note)
        } else {
            None
        };
        let nodes = note
            .as_ref()
            .map(|note| {
                note.document
                    .nodes()
                    .into_iter()
                    .filter(|node| !node.attrs.id.is_empty())
                    .map(|node| (node.attrs.id.as_str(), node))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let mut cached: Option<(EvidenceTarget, Vec<char>)> = None;
        let mut rows = chunks.query([&g.id])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let target: EvidenceTarget = serde_json::from_str(&row.get::<_, String>(1)?)?;
            let title: String = row.get(2)?;
            let text: String = row.get(3)?;
            let count: u32 = row.get(4)?;
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || target.kind != kind
                || target.object_id != g.object_id
                || target.content_revision != g.content_revision
                || target.metadata_revision != g.metadata_revision
                || title.trim().is_empty()
                || title.chars().count() > 300
                || text.len() > 64 * 1024
                || count == 0
                || count > profile.max_tokens
                || target.start_codepoint >= target.end_codepoint
                || (kind == EvidenceKind::Note
                    && (target.parse_id.is_some() || target.page_index.is_some()))
            {
                return Err(Error::IntegrityFailure);
            }
            if let Some((prior, _)) = &cached
                && prior.block_id == target.block_id
                && (prior.kind != target.kind
                    || prior.parse_id != target.parse_id
                    || prior.page_index != target.page_index)
            {
                // Reusing the text must never waive immutable locator identity checks.
                return Err(Error::IntegrityFailure);
            }
            if cached
                .as_ref()
                .is_none_or(|(prior, _)| prior.block_id != target.block_id)
            {
                let original = match kind {
                    EvidenceKind::Note => nodes
                        .get(target.block_id.as_str())
                        .map(|node| node.plain_text())
                        .ok_or(Error::IntegrityFailure)?,
                    EvidenceKind::Source => {
                        let found: Option<(String, Option<u32>, String)> = source_block
                            .query_row(
                                params![
                                    target.block_id,
                                    target.parse_id,
                                    target.content_revision,
                                    target.object_id
                                ],
                                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                            )
                            .optional()?;
                        let (text, page, parser) = found.ok_or(Error::IntegrityFailure)?;
                        if page != target.page_index || parser != g.parser_revision {
                            return Err(Error::IntegrityFailure);
                        }
                        text
                    }
                };
                cached = Some((target.clone(), original.chars().collect()));
            }
            let (_, characters) = cached.as_ref().ok_or(Error::IntegrityFailure)?;
            let original = characters
                .get(target.start_codepoint as usize..target.end_codepoint as usize)
                .ok_or(Error::IntegrityFailure)?;
            if !original.iter().copied().eq(text.chars()) {
                return Err(Error::IntegrityFailure);
            }
        }
    }
    Ok(())
}
pub(crate) fn publish(db: &Connection, request: &IndexPublish) -> Result<IndexGeneration> {
    let prior = generation(db, &request.generation_id)?;
    if prior.status == IndexGenerationStatus::Ready {
        let same: bool = db.query_row(
            "SELECT lease_token=?2 FROM index_generations WHERE id=?1",
            params![prior.id, request.lease_token],
            |r| r.get(0),
        )?;
        if same
            && prior.profile_id == request.profile_id
            && prior.chunk_count == request.chunk_count
        {
            return Ok(prior);
        }
        return Err(Error::InvalidInput("index publication replay mismatch"));
    }
    let g = check_lease(db, &request.generation_id, &request.lease_token)?;
    let count: u32 = db.query_row(
        "SELECT count(*) FROM index_chunks WHERE generation_id=?1",
        [&g.id],
        |r| r.get(0),
    )?;
    if g.profile_id != request.profile_id
        || g.chunk_count != request.chunk_count
        || count != g.chunk_count
    {
        return Err(Error::InvalidInput("index publication manifest mismatch"));
    }
    db.execute(
        "UPDATE index_generations SET status='ready',lease_expires_at=NULL WHERE id=?1",
        [&g.id],
    )?;
    db.execute("INSERT INTO active_index_generations(object_id,profile_id,generation_id) VALUES(?1,?2,?3) ON CONFLICT(object_id,profile_id) DO UPDATE SET generation_id=excluded.generation_id",params![g.object_id,g.profile_id,g.id])?;
    db.execute(
        "DELETE FROM index_dirty_objects WHERE object_id=?1",
        [&g.object_id],
    )?;
    let result = generation(db, &g.id)?;
    Ok(result)
}
impl Library {
    pub fn register_embedding_profile(
        &mut self,
        command_id: &str,
        spec: EmbeddingProfileSpec,
    ) -> Result<EmbeddingProfileMutation> {
        validate_profile_spec(&spec)?;
        let hash = mutations::request_hash(command_id, "search.profile", &spec)?;
        let json = serde_json::to_string(&spec)?;
        let id = format!("{:x}", Sha256::digest(json.as_bytes()));
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = mutations::replay(&tx, command_id, &hash, "search.profile")? {
            return Ok(old);
        }
        let previous: Option<String> = tx.query_row(
            "SELECT active_profile_id FROM search_settings WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        tx.execute("INSERT INTO embedding_profiles(id,spec_json,created_at) VALUES(?1,?2,?3) ON CONFLICT(id) DO NOTHING",params![id,json,Utc::now().to_rfc3339()])?;
        // Remember what the new profile replaced so the previous index can be
        // restored without re-embedding anything.
        tx.execute(
            "UPDATE search_settings SET previous_profile_id=active_profile_id WHERE singleton=1",
            [],
        )?;
        tx.execute(
            "UPDATE search_settings SET active_profile_id=?1 WHERE singleton=1",
            [&id],
        )?;
        if previous.as_deref() != Some(&id) {
            tx.execute("INSERT INTO index_dirty_objects(object_id) SELECT id FROM objects WHERE kind IN ('note','source') ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL",[])?;
        }
        let result = EmbeddingProfileMutation {
            profile: EmbeddingProfile {
                id: id.clone(),
                spec,
            },
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        mutations::store_receipt(&tx, &hash, "search.profile", &id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn active_embedding_profile(&self) -> Result<Option<EmbeddingProfile>> {
        let id: Option<String> = self.connection.query_row(
            "SELECT active_profile_id FROM search_settings WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        id.map(|id| profile(&self.connection, &id)).transpose()
    }
    pub fn index_generation(&self, id: &str) -> Result<IndexGeneration> {
        generation(&self.connection, id)
    }
    pub fn begin_index_generation(
        &mut self,
        command_id: &str,
        plan: IndexPlan,
    ) -> Result<IndexLease> {
        let hash = mutations::request_hash(command_id, "search.index_begin", &plan)?;
        if let Some(old) =
            mutations::replay(&self.connection, command_id, &hash, "search.index_begin")?
        {
            return Ok(old);
        }
        let input = self.index_input(&plan.object_id)?;
        if input.content_revision != plan.content_revision
            || input.metadata_revision != plan.metadata_revision
            || input.parser_revision != plan.parser_revision
        {
            return Err(Error::InvalidInput("index plan input changed"));
        }
        let profile = profile(&self.connection, &plan.profile_id)?;
        if plan.chunks.is_empty() || plan.chunks.len() > 100_000 {
            return Err(Error::InvalidInput("index chunk count exceeds bounds"));
        }
        let blocks = input
            .blocks
            .iter()
            .map(|b| (b.target.block_id.as_str(), b))
            .collect::<BTreeMap<_, _>>();
        let mut ranges: BTreeMap<&str, Vec<(u32, u32)>> = BTreeMap::new();
        let characters = blocks
            .iter()
            .map(|(id, block)| (*id, block.text.chars().collect::<Vec<_>>()))
            .collect::<BTreeMap<_, _>>();
        let mut seen = BTreeSet::new();
        let mut bytes = 0usize;
        let mut prepared = Vec::new();
        for chunk in &plan.chunks {
            if chunk.chunk_id.is_empty()
                || chunk.text.len() > 64 * 1024
                || chunk.chunk_id.len() > 128
                || !chunk
                    .chunk_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || !seen.insert(&chunk.chunk_id)
                || chunk.token_count == 0
                || chunk.token_count > profile.spec.max_tokens
            {
                return Err(Error::InvalidInput(
                    "invalid index chunk identity or token count",
                ));
            }
            let block = blocks
                .get(chunk.block_id.as_str())
                .ok_or(Error::InvalidInput("index chunk block not in input"))?;
            let chars = &characters[chunk.block_id.as_str()];
            if chunk.start_codepoint >= chunk.end_codepoint
                || chunk.end_codepoint as usize > chars.len()
            {
                return Err(Error::InvalidInput("index chunk offset out of bounds"));
            }
            let text = chars[chunk.start_codepoint as usize..chunk.end_codepoint as usize]
                .iter()
                .collect::<String>();
            if text != chunk.text {
                return Err(Error::InvalidInput("index chunk does not match input"));
            }
            bytes = bytes
                .checked_add(text.len())
                .ok_or(Error::InvalidInput("index data overflow"))?;
            if bytes > 64 * 1024 * 1024 {
                return Err(Error::InvalidInput("index chunks exceed 64 MiB"));
            }
            ranges
                .entry(&chunk.block_id)
                .or_default()
                .push((chunk.start_codepoint, chunk.end_codepoint));
            let mut target = block.target.clone();
            target.start_codepoint = chunk.start_codepoint;
            target.end_codepoint = chunk.end_codepoint;
            prepared.push((chunk, serde_json::to_string(&target)?));
        }
        for (id, block) in &blocks {
            if block.text.is_empty() {
                continue;
            }
            let intervals = ranges
                .get_mut(id)
                .ok_or(Error::InvalidInput("index plan omits a block"))?;
            intervals.sort_unstable();
            let mut covered = 0;
            for &(start, end) in intervals.iter() {
                if start > covered {
                    return Err(Error::InvalidInput("index plan has a text gap"));
                }
                covered = covered.max(end);
            }
            if covered as usize != characters[id].len() {
                return Err(Error::InvalidInput("index plan truncates input"));
            }
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = mutations::replay(&tx, command_id, &hash, "search.index_begin")? {
            return Ok(old);
        }
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT request_hash,lease_token FROM index_generations WHERE id=?1",
                [command_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let lease = if let Some((old, lease)) = prior {
            if old != hash {
                return Err(Error::InvalidInput("index command payload changed"));
            }
            lease
        } else {
            let lease = Uuid::new_v4().to_string();
            let now = Utc::now();
            tx.execute("INSERT INTO index_generations(id,object_id,content_revision,metadata_revision,profile_id,parser_revision,status,chunk_count,lease_token,lease_expires_at,created_at,request_hash) VALUES(?1,?2,?3,?4,?5,?6,'running',?7,?8,?9,?10,?11)",params![command_id,plan.object_id,plan.content_revision,plan.metadata_revision,plan.profile_id,plan.parser_revision,plan.chunks.len() as u32,lease,(now+Duration::minutes(5)).to_rfc3339(),now.to_rfc3339(),hash])?;
            lease
        };
        check_lease(&tx, command_id, &lease)?;
        tx.commit()?;
        let mut start = 0;
        while start < prepared.len() {
            let mut end = start;
            let mut bytes = 0;
            while end < prepared.len() && end - start < 100 {
                let next = prepared[end].0.text.len() + prepared[end].1.len() + input.title.len();
                if end > start && bytes + next > 256 * 1024 {
                    break;
                }
                bytes += next;
                end += 1;
            }
            let batch = &prepared[start..end];
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            check_lease(&tx, command_id, &lease)?;
            {
                let mut statement=tx.prepare("INSERT INTO index_chunks(id,generation_id,target_json,title,text,token_count) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(generation_id,id) DO NOTHING")?;
                for (chunk, target) in batch {
                    statement.execute(params![
                        chunk.chunk_id,
                        command_id,
                        target,
                        input.title,
                        chunk.text,
                        chunk.token_count
                    ])?;
                }
            }
            tx.commit()?;
            start = end;
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let generation = check_lease(&tx, command_id, &lease)?;
        let result = IndexLease {
            generation,
            lease_token: lease,
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        mutations::store_receipt(
            &tx,
            &hash,
            "search.index_begin",
            command_id,
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
    pub fn heartbeat_index_generation(&mut self, id: &str, lease: &str) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_lease(&tx, id, lease)?;
        tx.execute(
            "UPDATE index_generations SET lease_expires_at=?2 WHERE id=?1",
            params![id, (Utc::now() + Duration::minutes(5)).to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn publish_index_generation(&mut self, request: IndexPublish) -> Result<IndexGeneration> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = publish(&tx, &request)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn fail_index_generation(
        &mut self,
        id: &str,
        lease: &str,
        error_code: &str,
    ) -> Result<IndexGeneration> {
        if error_code.is_empty()
            || error_code.len() > 100
            || !error_code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(Error::InvalidInput("invalid index error code"));
        }
        if self.connection.execute("UPDATE index_generations SET status='failed',error_code=?3,lease_token=NULL,lease_expires_at=NULL WHERE id=?1 AND lease_token=?2 AND status='running'",params![id,lease,error_code])?!=1 {return Err(Error::InvalidInput("index failure lease is inactive"));}
        generation(&self.connection, id)
    }
    pub fn cancel_index_generation(&mut self, id: &str) -> Result<IndexGeneration> {
        self.connection.execute("UPDATE index_generations SET status='canceled',error_code='canceled',lease_token=NULL,lease_expires_at=NULL WHERE id=?1 AND status='running'",[id])?;
        generation(&self.connection, id)
    }
    pub fn recover_expired_index_generations(&mut self) -> Result<u32> {
        Ok(self.connection.execute("UPDATE index_generations SET status='failed',error_code='interrupted',lease_token=NULL,lease_expires_at=NULL WHERE status='running' AND lease_expires_at<=?1",[Utc::now().to_rfc3339()])? as u32)
    }
    /// Caller holds the physical workspace supervisor lease and has stopped old workers.
    pub fn recover_index_generations_exclusive(&mut self) -> Result<u32> {
        Ok(self.connection.execute("UPDATE index_generations SET status='failed',error_code='interrupted',lease_token=NULL,lease_expires_at=NULL WHERE status='running'",[])? as u32)
    }
    pub fn dirty_index_object_ids(&self, limit: u32) -> Result<Vec<String>> {
        crate::collections::validate_limit(limit)?;
        Ok(self.connection.prepare("SELECT d.object_id FROM index_dirty_objects d JOIN objects o ON o.id=d.object_id JOIN search_settings settings ON settings.singleton=1 LEFT JOIN notes n ON n.id=o.id LEFT JOIN sources s ON s.id=o.id LEFT JOIN source_active_parses p ON p.source_version_id=s.current_version_id WHERE settings.active_profile_id IS NOT NULL AND d.attempts<3 AND o.trashed=0 AND o.search_enabled=1 AND (n.id IS NOT NULL OR p.parse_id IS NOT NULL) AND (d.next_retry_at IS NULL OR d.next_retry_at<=?1) ORDER BY d.object_id LIMIT ?2")?.query_map(params![Utc::now().to_rfc3339(),limit],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }
    pub fn clear_dirty_index_input(
        &mut self,
        profile_id: &str,
        input: &IndexInput,
    ) -> Result<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: Option<String> = tx.query_row(
            "SELECT active_profile_id FROM search_settings WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if active.as_deref() != Some(profile_id)
            || !input_matches(
                &tx,
                &input.object_id,
                &input.content_revision,
                input.metadata_revision,
                &input.parser_revision,
            )?
        {
            return Ok(false);
        }
        let changed = tx.execute(
            "DELETE FROM index_dirty_objects WHERE object_id=?1",
            [&input.object_id],
        )? > 0;
        tx.commit()?;
        Ok(changed)
    }
    pub fn defer_dirty_index_objects(&mut self, ids: &[String], error_code: &str) -> Result<()> {
        if ids.len() > 100
            || error_code.is_empty()
            || error_code.len() > 100
            || !error_code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(Error::InvalidInput("invalid dirty index retry"));
        }
        for id in ids {
            crate::document::valid_uuid(id)?;
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for id in ids {
            let attempts: Option<u32> = tx
                .query_row(
                    "SELECT attempts FROM index_dirty_objects WHERE object_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(attempts) = attempts {
                let seconds = (30_i64 * (1_i64 << attempts.min(7))).min(3600);
                tx.execute("UPDATE index_dirty_objects SET attempts=?4,next_retry_at=?2,last_error=?3 WHERE object_id=?1",params![id,(Utc::now()+Duration::seconds(seconds)).to_rfc3339(),error_code,if deterministic_index_error(error_code) {3} else {(attempts+1).min(3)}])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn index_generations_page(
        &self,
        after_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<IndexGeneration>> {
        crate::collections::validate_limit(limit)?;
        let ids = self
            .connection
            .prepare(
                "SELECT id FROM index_generations WHERE (?1 IS NULL OR id>?1) ORDER BY id LIMIT ?2",
            )?
            .query_map(params![after_id, limit], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter()
            .map(|id| generation(&self.connection, id))
            .collect()
    }
    pub fn active_index_generation_ids(&self) -> Result<Vec<String>> {
        Ok(self
            .active_index_generations()?
            .into_iter()
            .map(|g| g.id)
            .collect())
    }
    pub fn index_status(&self) -> Result<IndexStatus> {
        Ok(IndexStatus {
            profile: self.active_embedding_profile()?,
            eligible_objects: crate::retrieval::eligible_ids(
                &self.connection,
                &SearchScope::default(),
            )?
            .len() as u32,
            ready_objects: self.active_index_generation_ids()?.len() as u32,
            running_generations: self.connection.query_row(
                "SELECT count(*) FROM index_generations WHERE status='running'",
                [],
                |r| r.get(0),
            )?,
        })
    }
    fn has_active_index_generation(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            &format!("SELECT EXISTS(SELECT 1 {ACTIVE_GENERATION_RELATIONS})"),
            [],
            |row| row.get(0),
        )?)
    }
    pub fn active_index_generations(&self) -> Result<Vec<IndexGeneration>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT g.id,g.object_id,g.content_revision,g.metadata_revision,
                    g.profile_id,g.parser_revision,g.chunk_count,g.error_code,g.created_at
             {ACTIVE_GENERATION_RELATIONS} ORDER BY g.id",
        ))?;
        Ok(statement
            .query_map([], |row| {
                Ok(IndexGeneration {
                    id: row.get(0)?,
                    object_id: row.get(1)?,
                    content_revision: row.get(2)?,
                    metadata_revision: row.get(3)?,
                    profile_id: row.get(4)?,
                    parser_revision: row.get(5)?,
                    status: IndexGenerationStatus::Ready,
                    chunk_count: row.get(6)?,
                    error_code: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn active_index_generations_in_scope(
        &self,
        scope: &SearchScope,
    ) -> Result<Vec<IndexGeneration>> {
        let _snapshot = if self.connection.is_autocommit() {
            Some(self.connection.unchecked_transaction()?)
        } else {
            None
        };
        let allowed = crate::retrieval::eligible_ids(&self.connection, scope)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        Ok(self
            .active_index_generations()?
            .into_iter()
            .filter(|g| allowed.contains(&g.object_id))
            .collect())
    }
    pub fn index_chunks_page(
        &self,
        generation_id: &str,
        after_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<IndexedChunk>> {
        crate::collections::validate_limit(limit)?;
        generation(&self.connection, generation_id)?;
        let rows=self.connection.prepare("SELECT id,target_json,title,text,token_count FROM index_chunks WHERE generation_id=?1 AND (?2 IS NULL OR id>?2) ORDER BY id LIMIT ?3")?.query_map(params![generation_id,after_id,limit],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,u32>(4)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|(id, target, title, text, token_count)| {
                Ok(IndexedChunk {
                    id,
                    generation_id: generation_id.into(),
                    target: serde_json::from_str(&target)?,
                    title,
                    text,
                    token_count,
                })
            })
            .collect()
    }
    pub fn search_with_vectors(
        &self,
        request: &SearchQuery,
        candidates: Vec<VectorCandidate>,
    ) -> Result<SearchResults> {
        self.search_with_annotation_vectors(request, candidates, vec![])
    }
    pub fn search_with_annotation_vectors(
        &self,
        request: &SearchQuery,
        candidates: Vec<VectorCandidate>,
        annotations: Vec<AnnotationVectorCandidate>,
    ) -> Result<SearchResults> {
        let _snapshot = if self.connection.is_autocommit() {
            Some(self.connection.unchecked_transaction()?)
        } else {
            None
        };
        request.validate()?;
        let mut lexical = if request.mode == SearchMode::Semantic {
            SearchResults {
                items: vec![],
                lexical_available: true,
                semantic_available: false,
                unavailable_reason: None,
                profile_id: None,
            }
        } else {
            self.keyword_search(request)?
        };
        if request.mode == SearchMode::Lexical {
            return Ok(lexical);
        }
        if candidates.len() > 2000 || annotations.len() > 2000 {
            return Err(Error::InvalidInput("invalid vector retrieval bounds"));
        }
        let Some(profile) = self.active_embedding_profile()? else {
            lexical.unavailable_reason = Some("embedding_unconfigured".into());
            return Ok(lexical);
        };
        lexical.profile_id = Some(profile.id.clone());
        lexical.semantic_available = self.has_active_index_generation()?;
        let mut vectors = vec![];
        let mut seen = BTreeSet::new();
        let mut candidates = candidates;
        candidates.sort_by(|a, b| b.cosine_similarity.total_cmp(&a.cosine_similarity));
        for candidate in candidates {
            if !candidate.cosine_similarity.is_finite()
                || !(-1.0..=1.0).contains(&candidate.cosine_similarity)
            {
                return Err(Error::InvalidInput("invalid cosine similarity"));
            }
            if candidate.profile_id != profile.id
                || request
                    .minimum_cosine
                    .is_some_and(|v| candidate.cosine_similarity < v)
            {
                continue;
            }
            let row:Option<(String,String,String)>=self.connection.query_row("SELECT c.target_json,c.title,c.text FROM index_chunks c JOIN index_generations g ON g.id=c.generation_id JOIN active_index_generations a ON a.generation_id=g.id AND a.profile_id=g.profile_id AND a.object_id=g.object_id WHERE c.id=?1 AND c.generation_id=?2 AND g.object_id=?3 AND g.content_revision=?4 AND g.profile_id=?5 AND g.status='ready'",params![candidate.chunk_id,candidate.generation_id,candidate.object_id,candidate.content_revision,profile.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let Some((target, title, text)) = row else {
                continue;
            };
            let evidence = SearchEvidence {
                annotation: None,
                id: candidate.chunk_id,
                target: serde_json::from_str(&target)?,
                title,
                text,
                lexical_rank: None,
                cosine_similarity: Some(candidate.cosine_similarity),
                fusion_rank: None,
                generation_id: Some(candidate.generation_id),
            };
            if crate::retrieval::validate_evidence(&self.connection, &request.scope, &evidence)?
                && seen.insert((evidence.generation_id.clone(), evidence.id.clone()))
            {
                vectors.push(evidence);
            }
        }
        let mut annotations = annotations;
        annotations.sort_by(|a, b| b.cosine_similarity.total_cmp(&a.cosine_similarity));
        let mut annotation_seen = BTreeSet::new();
        for candidate in annotations {
            if !candidate.cosine_similarity.is_finite()
                || !(-1.0..=1.0).contains(&candidate.cosine_similarity)
            {
                return Err(Error::InvalidInput("invalid cosine similarity"));
            }
            if candidate.profile_id != profile.id || candidate.evidence.generation_id.is_some() {
                continue;
            }
            let mut evidence = candidate.evidence;
            let Some(annotation) = evidence.annotation.as_mut() else {
                continue;
            };
            if annotation
                .embedding_profile_id
                .as_ref()
                .is_some_and(|id| id != &profile.id)
            {
                continue;
            }
            annotation.embedding_profile_id = Some(profile.id.clone());
            evidence.cosine_similarity = Some(candidate.cosine_similarity);
            evidence.lexical_rank = None;
            evidence.fusion_rank = None;
            if !crate::retrieval::validate_evidence(&self.connection, &request.scope, &evidence)? {
                continue;
            }
            lexical.semantic_available = true;
            if request
                .minimum_cosine
                .is_none_or(|v| candidate.cosine_similarity >= v)
                && annotation_seen.insert(evidence_fusion_key(&evidence)?)
            {
                vectors.push(evidence);
            }
        }
        if !lexical.semantic_available {
            lexical.unavailable_reason = Some("index_unavailable".into());
            return Ok(lexical);
        }
        lexical.unavailable_reason = None;
        vectors.sort_by(|a, b| {
            b.cosine_similarity
                .unwrap_or(-1.0)
                .total_cmp(&a.cosine_similarity.unwrap_or(-1.0))
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut combined: BTreeMap<String, SearchEvidence> = BTreeMap::new();
        for (rank, mut evidence) in lexical.items.drain(..).enumerate() {
            evidence.fusion_rank = Some(1.0 / (61 + rank) as f64);
            combined.insert(evidence_fusion_key(&evidence)?, evidence);
        }
        for (rank, mut evidence) in vectors.into_iter().enumerate() {
            let key = evidence_fusion_key(&evidence)?;
            let score = 1.0 / (61 + rank) as f64;
            if let Some(existing) = combined.get_mut(&key) {
                existing.cosine_similarity = evidence.cosine_similarity;
                existing.fusion_rank = Some(existing.fusion_rank.unwrap_or(0.0) + score);
                existing.generation_id = evidence.generation_id;
                existing.id = evidence.id;
                existing.annotation = evidence.annotation;
            } else {
                evidence.fusion_rank = Some(score);
                combined.insert(key, evidence);
            }
        }
        let mut items = combined.into_values().collect::<Vec<_>>();
        items.sort_by(|a, b| {
            b.fusion_rank
                .unwrap_or(0.0)
                .total_cmp(&a.fusion_rank.unwrap_or(0.0))
                .then_with(|| a.id.cmp(&b.id))
        });
        items.truncate(request.limit as usize);
        lexical.items = items;
        Ok(lexical)
    }
}

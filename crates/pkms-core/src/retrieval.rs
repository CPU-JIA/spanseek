//! Snapshot-bound lexical retrieval and evidence verification.
use crate::{
    Error, EvidenceKind, EvidenceTarget, IndexInput, IndexInputBlock, Library, Result,
    SearchEvidence, SearchQuery, SearchResults, SearchScope,
};
use rusqlite::{Connection, OptionalExtension, params};

fn id(value: &str) -> Result<()> {
    if uuid::Uuid::parse_str(value).is_err() {
        return Err(Error::InvalidInput("invalid search scope ID"));
    }
    Ok(())
}
fn enum_string<T: serde::Serialize>(value: Option<T>) -> Result<Option<String>> {
    value
        .map(|v| {
            serde_json::to_value(v)?
                .as_str()
                .map(str::to_owned)
                .ok_or(Error::IntegrityFailure)
        })
        .transpose()
}
fn validate_scope(scope: &SearchScope) -> Result<()> {
    if scope.object_ids.len() > 100 || scope.tag_ids.len() > 100 || scope.kinds.len() > 2 {
        return Err(Error::InvalidInput("search scope exceeds bounds"));
    }
    for value in scope
        .object_ids
        .iter()
        .chain(&scope.tag_ids)
        .chain(scope.collection_id.iter())
    {
        id(value)?;
    }
    Ok(())
}

impl SearchQuery {
    /// Validate request bounds before remote work, without executing a search.
    /// Result publication still checks current scope, lifecycle and revisions.
    pub fn validate(&self) -> Result<()> {
        if self.query.len() > 4096 || !(1..=100).contains(&self.limit) {
            return Err(Error::InvalidInput("query or limit exceeds bounds"));
        }
        validate_scope(&self.scope)?;
        if self.mode != crate::SearchMode::Lexical
            && self
                .minimum_cosine
                .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
        {
            return Err(Error::InvalidInput("invalid vector retrieval bounds"));
        }
        Ok(())
    }
}

pub(crate) fn eligible_ids(db: &Connection, scope: &SearchScope) -> Result<Vec<String>> {
    validate_scope(scope)?;
    let objects = serde_json::to_string(&scope.object_ids)?;
    let tags = serde_json::to_string(&scope.tag_ids)?;
    let kinds = serde_json::to_string(&scope.kinds)?;
    // A bound OR (empty scope OR id IN ...) forces a full object scan even
    // when validating one candidate. Keep the selective predicate explicit.
    let object_filter = if scope.object_ids.is_empty() {
        ""
    } else {
        " AND o.id IN (SELECT value FROM json_each(?1))"
    };
    let mut stmt=db.prepare(&format!("SELECT o.id FROM objects o LEFT JOIN notes n ON n.id=o.id LEFT JOIN sources s ON s.id=o.id LEFT JOIN source_versions v ON v.id=s.current_version_id LEFT JOIN reading_states r ON r.source_id=s.id WHERE o.kind IN ('note','source') AND o.trashed=0 AND o.search_enabled=1 AND (o.kind!='note' OR n.trashed=0){object_filter} AND (json_array_length(?2)=0 OR o.kind IN (SELECT value FROM json_each(?2))) AND NOT EXISTS (SELECT 1 FROM json_each(?3) t WHERE NOT EXISTS (SELECT 1 FROM object_tags ot WHERE ot.object_id=o.id AND ot.tag_id=t.value)) AND (?4 IS NULL OR EXISTS(SELECT 1 FROM memberships m JOIN objects c ON c.id=m.collection_id WHERE m.member_id=o.id AND m.collection_id=?4 AND c.trashed=0)) AND (?5 IS NULL OR o.inbox_state=?5) AND (?6 IS NULL OR (o.kind='source' AND coalesce(r.status,'unread')=?6)) AND (?7 IS NULL OR v.format=?7) AND (?8 IS NULL OR o.archived=?8) ORDER BY o.id"))?;
    Ok(stmt
        .query_map(
            params![
                objects,
                kinds,
                tags,
                scope.collection_id,
                scope.inbox_state.map(|v| v.as_str()),
                enum_string(scope.reading_status.as_ref())?,
                enum_string(scope.source_format)?,
                scope.archived
            ],
            |r| r.get(0),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

/// True when the scope admits every indexed object, so the caller can push the
/// object predicate into SQL instead of materializing every eligible id. The
/// default scope (the search page and the answer pipeline) always does.
fn scope_admits_every_object(scope: &SearchScope) -> bool {
    scope.object_ids.is_empty()
        && scope.kinds.is_empty()
        && scope.tag_ids.is_empty()
        && scope.collection_id.is_none()
        && scope.inbox_state.is_none()
        && scope.reading_status.is_none()
        && scope.source_format.is_none()
        && scope.archived.is_none()
}

/// A term matching at least this many documents *and* at least
/// [`BOILERPLATE_RATIO`] times as many as the rarest query term cannot separate
/// documents: it is boilerplate for this query (a template word, a header that
/// repeats in every document). Scoring its postings costs time per matched
/// document while the ranked list holds a handful of items.
const BOILERPLATE_MINIMUM_DOCUMENTS: i64 = 32;
const BOILERPLATE_RATIO: i64 = 8;

/// Indexes of the query terms that still separate documents. The rarest term
/// always survives, so pruning can never turn a query into a no-op.
pub(crate) fn discriminative_indexes(frequencies: &[Option<i64>]) -> Vec<usize> {
    let Some(rarest) = frequencies.iter().flatten().min().copied() else {
        return (0..frequencies.len()).collect();
    };
    (0..frequencies.len())
        .filter(|index| {
            frequencies[*index].is_none_or(|documents| {
                documents < BOILERPLATE_MINIMUM_DOCUMENTS
                    || documents <= rarest.saturating_mul(BOILERPLATE_RATIO)
            })
        })
        .collect()
}

/// Document frequency of one token in one FTS table. `None` means the token is
/// absent from the index.
///
/// The count walks the postings list, and a cheaper capped probe was measured and
/// rejected: the cap saturates the very terms the pruning rule has to recognise as
/// boilerplate, which stops them from being pruned at all (the "带 保留" query then
/// matched the whole library again). An exact count of a 10,000 document term costs
/// about 1 ms, which is cheap next to the ranked query it decides.
fn term_documents(db: &Connection, table: &str, term: &str) -> Result<Option<i64>> {
    // `table` is one of the internal FTS literals below, never user input.
    let sql = format!("SELECT count(*) FROM {table} WHERE {table} MATCH ?1");
    let count: i64 = db.query_row(&sql, [format!("\"{}\"", term.replace('"', "\"\""))], |r| {
        r.get(0)
    })?;
    Ok((count > 0).then_some(count))
}

/// The FTS match expression for the notes channel: the query terms minus the
/// ones that cannot separate documents. The dropped terms still count for the
/// coverage ranking that orders the returned evidence.
fn match_expression(db: &Connection, terms: &[String]) -> Result<String> {
    let mut frequencies = Vec::with_capacity(terms.len());
    for term in terms {
        frequencies.push(term_documents(db, "note_fts", term)?);
    }
    Ok(discriminative_indexes(&frequencies)
        .into_iter()
        .map(|index| format!("\"{}\"", terms[index].replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR "))
}

/// Document frequency of one token over *sources*.
///
/// The source channels match many blocks per document, so counting rows would
/// call a term boilerplate merely because one long document repeats it. What the
/// rule has to recognise is a term that cannot separate sources, so this counts
/// distinct sources.
pub(crate) fn source_term_documents(
    db: &Connection,
    table: &str,
    term: &str,
) -> Result<Option<i64>> {
    // `table` is one of the internal FTS literals below, never user input.
    let sql = format!("SELECT count(DISTINCT source_id) FROM {table} WHERE {table} MATCH ?1");
    let count: i64 = db.query_row(&sql, [format!("\"{}\"", term.replace('"', "\"\""))], |r| {
        r.get(0)
    })?;
    Ok((count > 0).then_some(count))
}

/// The body match for the source channel: the query terms that can still separate
/// sources, by the same rule the notes channel has used since V18-G02. The dropped
/// terms keep counting for the coverage ranking that orders the returned evidence,
/// and the rarest term always survives.
pub(crate) fn source_match_expression(db: &Connection, terms: &[String]) -> Result<String> {
    let mut frequencies = Vec::with_capacity(terms.len());
    for term in terms {
        frequencies.push(source_term_documents(db, "source_fts", term)?);
    }
    Ok(discriminative_indexes(&frequencies)
        .into_iter()
        .map(|index| format!("\"{}\"", terms[index].replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR "))
}

pub(crate) fn read_index_input(db: &Connection, object_id: &str) -> Result<IndexInput> {
    let (kind, metadata): (String, u32) = db
        .query_row(
            "SELECT kind,row_version FROM objects WHERE id=?1 AND trashed=0 AND search_enabled=1",
            [object_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if kind == "note" {
        let revision: String = db
            .query_row(
                "SELECT current_revision_id FROM notes WHERE id=?1 AND trashed=0",
                [object_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let note = crate::notes::revision(db, object_id, &revision)?;
        // Container nodes overlap their descendants. Index only the innermost real blocks.
        let blocks = note
            .document
            .nodes()
            .into_iter()
            .filter(|n| {
                !n.attrs.id.is_empty() && !n.content.iter().any(|child| !child.attrs.id.is_empty())
            })
            .filter_map(|node| {
                let text = node.plain_text();
                if text.is_empty() {
                    return None;
                }
                Some(IndexInputBlock {
                    target: EvidenceTarget {
                        object_id: object_id.into(),
                        kind: EvidenceKind::Note,
                        content_revision: revision.clone(),
                        metadata_revision: metadata,
                        parse_id: None,
                        block_id: node.attrs.id.clone(),
                        start_codepoint: 0,
                        end_codepoint: text.chars().count() as u32,
                        page_index: None,
                    },
                    text,
                })
            })
            .collect();
        Ok(IndexInput {
            object_id: object_id.into(),
            kind: EvidenceKind::Note,
            content_revision: revision,
            metadata_revision: metadata,
            parser_revision: format!("note-document-{}", note.schema_version),
            title: note.title,
            blocks,
        })
    } else if kind == "source" {
        let (title,version,parse,parser):(String,String,String,String)=db.query_row("SELECT s.display_title,s.current_version_id,a.parse_id,p.parser_version FROM sources s JOIN source_active_parses a ON a.source_version_id=s.current_version_id JOIN source_parses p ON p.id=a.parse_id WHERE s.id=?1",[object_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or(Error::NotFound)?;
        let rows = db
            .prepare(
                "SELECT id,text,page_index FROM source_blocks WHERE parse_id=?1 ORDER BY position",
            )?
            .query_map([&parse], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<u32>>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let blocks = rows
            .into_iter()
            .filter(|(_, text, _)| !text.is_empty())
            .map(|(block, text, page)| IndexInputBlock {
                target: EvidenceTarget {
                    object_id: object_id.into(),
                    kind: EvidenceKind::Source,
                    content_revision: version.clone(),
                    metadata_revision: metadata,
                    parse_id: Some(parse.clone()),
                    block_id: block,
                    start_codepoint: 0,
                    end_codepoint: text.chars().count() as u32,
                    page_index: page,
                },
                text,
            })
            .collect();
        Ok(IndexInput {
            object_id: object_id.into(),
            kind: EvidenceKind::Source,
            content_revision: version,
            metadata_revision: metadata,
            parser_revision: parser,
            title,
            blocks,
        })
    } else {
        Err(Error::NotFound)
    }
}

fn excerpt(block: &IndexInputBlock, query: &[String]) -> (EvidenceTarget, String) {
    let chars: Vec<char> = block.text.chars().collect();
    let maximum_start = chars.len().saturating_sub(2048);
    let mut start = 0;
    if maximum_start > 0 {
        // Each complete matching token defines an interval of valid window starts.
        // Sweep those intervals to maximize distinct query coverage, rather than
        // letting an early question word hide the relevant facts later in a block.
        // Token offsets refer to original codepoints, including expanding lowercase.
        let mut events = Vec::new();
        for token in crate::tokenizer().cut_for_search(&block.text, true) {
            let Ok(term) = query.binary_search(&token.word.to_lowercase()) else {
                continue;
            };
            let first = token.end.saturating_sub(2048);
            let last = token.start.min(maximum_start);
            if first <= last {
                events.push((first, term, 1_i32));
                events.push((last + 1, term, -1_i32));
            }
        }
        events.sort_unstable_by_key(|event| event.0);
        let mut counts = vec![0_i32; query.len()];
        let mut covered = 0;
        let mut best = 0;
        let mut i = 0;
        while i < events.len() {
            let position = events[i].0;
            while i < events.len() && events[i].0 == position {
                let (_, term, delta) = events[i];
                covered -= usize::from(counts[term] > 0);
                counts[term] += delta;
                covered += usize::from(counts[term] > 0);
                i += 1;
            }
            if covered > best && position <= maximum_start {
                best = covered;
                let last = events.get(i).map_or(maximum_start, |event| event.0 - 1);
                let last = last.min(maximum_start);
                start = if last == maximum_start {
                    last
                } else {
                    last.saturating_sub(128).max(position)
                };
            }
        }
    }
    let end = (start + 2048).min(chars.len());
    let mut target = block.target.clone();
    target.start_codepoint = block.target.start_codepoint + start as u32;
    target.end_codepoint = block.target.start_codepoint + end as u32;
    (target, chars[start..end].iter().collect())
}

impl Library {
    pub fn index_input(&self, object_id: &str) -> Result<IndexInput> {
        id(object_id)?;
        let tx = self.connection.unchecked_transaction()?;
        let result = read_index_input(&tx, object_id)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn index_object_ids(&self, after_id: Option<&str>, limit: u32) -> Result<Vec<String>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput("invalid index page limit"));
        }
        if let Some(after) = after_id {
            id(after)?;
        }
        Ok(self.connection.prepare("SELECT o.id FROM objects o WHERE o.kind IN ('note','source') AND o.trashed=0 AND o.search_enabled=1 AND (?1 IS NULL OR o.id>?1) AND (o.kind='note' OR EXISTS(SELECT 1 FROM sources s JOIN source_active_parses a ON a.source_version_id=s.current_version_id WHERE s.id=o.id)) ORDER BY o.id LIMIT ?2")?.query_map(params![after_id,limit],|r|r.get(0))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }
    pub fn keyword_search(&self, request: &SearchQuery) -> Result<SearchResults> {
        request.validate()?;
        let tx = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        // The eligible set is materialized only when the scope really filters
        // something. The default scope admits every indexed object, and building
        // that id list plus its JSON round trip was about half of a large-library
        // search; the FTS predicates below decide the same thing in SQL.
        let eligible = if scope_admits_every_object(&request.scope) {
            None
        } else {
            Some(eligible_ids(&self.connection, &request.scope)?)
        };
        let eligible_json = eligible.as_ref().map(serde_json::to_string).transpose()?;
        let terms = crate::tokens(&request.query);
        let mut items = Vec::new();
        if !terms.is_empty() && eligible.as_ref().is_none_or(|ids| !ids.is_empty()) {
            // Ranked retrieval accepts natural-language questions: absent question
            // words must not veto matching evidence. BM25 rewards informative terms
            // and multiple matches without a corpus-specific stop-word dictionary.
            // The older explicit keyword search APIs retain their AND semantics.
            let expression = match_expression(&self.connection, &terms)?;
            // Exactly one ordering key, so FTS5 itself consumes the rank order
            // (plan `SCAN f VIRTUAL TABLE INDEX rM3`, no `USE TEMP B-TREE FOR
            // ORDER BY`): it scores in rank order and stops once the limit is
            // satisfied instead of materializing every matching row and sorting
            // it with the joined columns. `rank MATCH 'bm25(0.0,3.0,1.0)'` sets
            // the same weighted rank function the old ORDER BY expression named,
            // so the scores are unchanged; ties now fall back to insertion order
            // (rowid) instead of the note UUID.
            let note_ids=self.connection.prepare("SELECT f.note_id FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=f.note_id WHERE note_fts MATCH ?1 AND rank MATCH 'bm25(0.0,3.0,1.0)' AND n.trashed=0 AND o.trashed=0 AND o.search_enabled=1 AND o.kind='note' AND (?2 IS NULL OR f.note_id IN (SELECT value FROM json_each(?2))) ORDER BY rank LIMIT ?3")?.query_map(params![expression,eligible_json.as_deref(),request.limit],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
            let mut notes = Vec::new();
            for object in note_ids {
                let input = read_index_input(&self.connection, &object)?;
                let block = input.blocks.iter().max_by_key(|b| {
                    let matched = crate::tokens(&b.text);
                    terms
                        .iter()
                        .filter(|term| matched.binary_search(term).is_ok())
                        .count()
                });
                if let Some(block) = block {
                    let (target, text) = excerpt(block, &terms);
                    notes.push(SearchEvidence {
                        annotation: None,
                        id: format!(
                            "lexical:{}:{}:{}",
                            object, target.block_id, target.start_codepoint
                        ),
                        target,
                        title: input.title,
                        text,
                        lexical_rank: None,
                        cosine_similarity: None,
                        fusion_rank: None,
                        generation_id: None,
                    });
                }
            }
            let hits =
                self.search_sources_ranked(&request.query, request.limit, eligible.as_deref())?;
            let mut sources = Vec::new();
            for hit in hits {
                let (metadata,parse):(u32,String)=self.connection.query_row("SELECT o.row_version,a.parse_id FROM objects o JOIN source_active_parses a ON a.source_version_id=?2 WHERE o.id=?1",params![hit.source_id,hit.source_version_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
                let block = IndexInputBlock {
                    target: EvidenceTarget {
                        object_id: hit.source_id.clone(),
                        kind: EvidenceKind::Source,
                        content_revision: hit.source_version_id,
                        metadata_revision: metadata,
                        parse_id: Some(parse),
                        block_id: hit.block_id,
                        start_codepoint: 0,
                        end_codepoint: hit.text.chars().count() as u32,
                        page_index: hit.page_index,
                    },
                    text: hit.text,
                };
                let (target, text) = excerpt(&block, &terms);
                sources.push(SearchEvidence {
                    annotation: None,
                    id: format!(
                        "lexical:{}:{}:{}",
                        hit.source_id, target.block_id, target.start_codepoint
                    ),
                    target,
                    title: hit.title,
                    text,
                    lexical_rank: None,
                    cosine_similarity: None,
                    fusion_rank: None,
                    generation_id: None,
                });
            }
            // Preserve each index's relevance order. Raw BM25 scores across
            // indexes are incomparable; merge their heads using query coverage
            // in the title and actual evidence block, interleaving equal coverage.
            let coverage = |item: SearchEvidence| {
                let matched = crate::tokens(&format!("{} {}", item.title, item.text));
                let count = terms
                    .iter()
                    .filter(|term| matched.binary_search(term).is_ok())
                    .count();
                (count, item)
            };
            let mut notes = notes.into_iter().map(coverage).peekable();
            let mut sources = sources.into_iter().map(coverage).peekable();
            loop {
                match (notes.peek(), sources.peek()) {
                    (None, None) => break,
                    (Some(_), None) => items.extend(notes.next().map(|(_, item)| item)),
                    (None, Some(_)) => items.extend(sources.next().map(|(_, item)| item)),
                    (Some((n, _)), Some((s, _))) => match n.cmp(s) {
                        std::cmp::Ordering::Greater => {
                            items.extend(notes.next().map(|(_, item)| item))
                        }
                        std::cmp::Ordering::Less => {
                            items.extend(sources.next().map(|(_, item)| item))
                        }
                        std::cmp::Ordering::Equal => {
                            items.extend(notes.next().map(|(_, item)| item));
                            items.extend(sources.next().map(|(_, item)| item));
                        }
                    },
                }
            }
            items.truncate(request.limit as usize);
            for (rank, item) in items.iter_mut().enumerate() {
                item.lexical_rank = Some(rank as u32 + 1);
            }
        }
        if !terms.is_empty() {
            let coverage = |text: &str| {
                let matched = crate::tokens(text);
                terms
                    .iter()
                    .filter(|term| matched.binary_search(term).is_ok())
                    .count()
            };
            let mut annotations = Vec::new();
            for mut evidence in self.annotation_search_inputs(&request.scope)? {
                let count = coverage(&evidence.text);
                if count == 0 {
                    continue;
                }
                let block = IndexInputBlock {
                    target: evidence.target.clone(),
                    text: evidence.text,
                };
                let (target, text) = excerpt(&block, &terms);
                evidence.id = format!(
                    "{}:{}:{}",
                    evidence.id, target.start_codepoint, target.end_codepoint
                );
                evidence.target = target;
                evidence.text = text;
                annotations.push((count, evidence));
            }
            annotations.sort_by(|(a, left), (b, right)| b.cmp(a).then(left.id.cmp(&right.id)));
            if !annotations.is_empty() {
                let mut existing = std::mem::take(&mut items)
                    .into_iter()
                    .map(|item| (coverage(&format!("{} {}", item.title, item.text)), item))
                    .peekable();
                let mut annotations = annotations.into_iter().peekable();
                while items.len() < request.limit as usize {
                    match (existing.peek(), annotations.peek()) {
                        (None, None) => break,
                        (Some(_), None) => items.extend(existing.next().map(|(_, item)| item)),
                        (None, Some(_)) => items.extend(annotations.next().map(|(_, item)| item)),
                        (Some((a, _)), Some((b, _))) if a >= b => {
                            items.extend(existing.next().map(|(_, item)| item))
                        }
                        _ => items.extend(annotations.next().map(|(_, item)| item)),
                    }
                }
            }
            for (rank, item) in items.iter_mut().enumerate() {
                item.lexical_rank = Some(rank as u32 + 1);
            }
        }
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(SearchResults {
            items,
            lexical_available: true,
            semantic_available: false,
            unavailable_reason: None,
            profile_id: None,
        })
    }
    /// Public, read-only revalidation used by cited outputs. The original search
    /// scope is intentionally not accepted here: the output must still resolve
    /// when the object, revision, block and excerpt remain unchanged.
    pub fn validate_search_evidence(
        &self,
        evidence: &SearchEvidence,
        scope: &SearchScope,
    ) -> Result<bool> {
        validate_evidence(&self.connection, scope, evidence)
    }
}
type SourceEvidenceRow = (String, u32, String, String, String, String, Option<u32>);

pub(crate) fn validate_evidence(
    db: &Connection,
    scope: &SearchScope,
    evidence: &SearchEvidence,
) -> Result<bool> {
    if evidence.annotation.is_some() {
        return crate::annotation_retrieval::validate_annotation_evidence(
            db, scope, evidence, false,
        );
    }
    if scope.object_ids.len() > 100 || scope.tag_ids.len() > 100 || scope.kinds.len() > 2 {
        return Err(Error::InvalidInput("search scope exceeds bounds"));
    }
    for value in scope
        .object_ids
        .iter()
        .chain(&scope.tag_ids)
        .chain(scope.collection_id.iter())
    {
        id(value)?;
    }
    let t = &evidence.target;
    if uuid::Uuid::parse_str(&t.object_id).is_err() {
        return Ok(false);
    }
    if !scope.object_ids.is_empty() && !scope.object_ids.contains(&t.object_id) {
        return Ok(false);
    }
    let mut narrowed = scope.clone();
    narrowed.object_ids = vec![t.object_id.clone()];
    if eligible_ids(db, &narrowed)?.is_empty() {
        return Ok(false);
    }
    let (kind, revision, metadata, title, parser, block) = if t.kind == EvidenceKind::Source {
        let row:Option<SourceEvidenceRow>=db.query_row("SELECT s.current_version_id,o.row_version,s.display_title,p.parser_version,a.parse_id,b.text,b.page_index FROM sources s JOIN objects o ON o.id=s.id JOIN source_active_parses a ON a.source_version_id=s.current_version_id JOIN source_parses p ON p.id=a.parse_id JOIN source_blocks b ON b.parse_id=a.parse_id AND b.id=?2 WHERE s.id=?1",params![t.object_id,t.block_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
        let Some((revision, metadata, title, parser, parse, text, page)) = row else {
            return Ok(false);
        };
        let block = IndexInputBlock {
            target: EvidenceTarget {
                object_id: t.object_id.clone(),
                kind: EvidenceKind::Source,
                content_revision: revision.clone(),
                metadata_revision: metadata,
                parse_id: Some(parse),
                block_id: t.block_id.clone(),
                start_codepoint: 0,
                end_codepoint: text.chars().count() as u32,
                page_index: page,
            },
            text,
        };
        (
            EvidenceKind::Source,
            revision,
            metadata,
            title,
            parser,
            block,
        )
    } else {
        let input = match read_index_input(db, &t.object_id) {
            Ok(v) => v,
            Err(Error::NotFound) => return Ok(false),
            Err(e) => return Err(e),
        };
        let Some(block) = input
            .blocks
            .into_iter()
            .find(|b| b.target.block_id == t.block_id)
        else {
            return Ok(false);
        };
        (
            input.kind,
            input.content_revision,
            input.metadata_revision,
            input.title,
            input.parser_revision,
            block,
        )
    };
    if kind != t.kind
        || revision != t.content_revision
        || metadata != t.metadata_revision
        || title != evidence.title
        || block.target.parse_id != t.parse_id
        || block.target.page_index != t.page_index
        || t.start_codepoint >= t.end_codepoint
        || t.end_codepoint > block.target.end_codepoint
    {
        return Ok(false);
    }
    let text: String = block
        .text
        .chars()
        .skip(t.start_codepoint as usize)
        .take((t.end_codepoint - t.start_codepoint) as usize)
        .collect();
    if text != evidence.text {
        return Ok(false);
    }
    if let Some(generation) = &evidence.generation_id {
        let stored:Option<(String,String,String)>=db.query_row("SELECT c.target_json,c.title,c.text FROM index_chunks c JOIN index_generations g ON g.id=c.generation_id JOIN active_index_generations a ON a.generation_id=g.id AND a.object_id=g.object_id AND a.profile_id=g.profile_id JOIN search_settings s ON s.active_profile_id=g.profile_id WHERE c.id=?1 AND g.id=?2 AND g.status='ready' AND g.object_id=?3 AND g.content_revision=?4 AND g.metadata_revision=?5 AND g.parser_revision=?6",params![evidence.id,generation,t.object_id,t.content_revision,t.metadata_revision,parser],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((target, title, text)) = stored else {
            return Ok(false);
        };
        if serde_json::from_str::<serde_json::Value>(&target)? != serde_json::to_value(t)?
            || title != evidence.title
            || text != evidence.text
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query() -> SearchQuery {
        SearchQuery {
            query: "证据".into(),
            mode: crate::SearchMode::Lexical,
            scope: SearchScope::default(),
            limit: 10,
            minimum_cosine: None,
        }
    }
    #[test]
    fn evidence_rejects_forgery_metadata_lifecycle_and_revision_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Library::open(dir.path()).unwrap();
        let note = db.create_note("真实证据", "这里是证据😀全文").unwrap();
        let scope = SearchScope::default();
        let tx = db.connection.unchecked_transaction().unwrap();
        let original = db.keyword_search(&query()).unwrap().items.remove(0);
        assert!(!db.connection.is_autocommit());
        tx.rollback().unwrap();
        assert!(validate_evidence(&db.connection, &scope, &original).unwrap());
        let mut forged = original.clone();
        forged.text = "伪造证据".into();
        assert!(!validate_evidence(&db.connection, &scope, &forged).unwrap());
        forged = original.clone();
        forged.target.end_codepoint += 1;
        assert!(!validate_evidence(&db.connection, &scope, &forged).unwrap());
        forged = original.clone();
        forged.target.parse_id = Some(uuid::Uuid::new_v4().to_string());
        assert!(!validate_evidence(&db.connection, &scope, &forged).unwrap());
        let restricted = SearchScope {
            object_ids: vec![uuid::Uuid::new_v4().to_string()],
            ..Default::default()
        };
        assert!(!validate_evidence(&db.connection, &restricted, &original).unwrap());
        db.connection
            .execute(
                "UPDATE objects SET search_enabled=0 WHERE id=?1",
                [&note.id],
            )
            .unwrap();
        assert!(!validate_evidence(&db.connection, &scope, &original).unwrap());
        db.connection
            .execute(
                "UPDATE objects SET search_enabled=1,trashed=1 WHERE id=?1",
                [&note.id],
            )
            .unwrap();
        assert!(!validate_evidence(&db.connection, &scope, &original).unwrap());
        db.connection
            .execute(
                "UPDATE objects SET trashed=0,row_version=row_version+1 WHERE id=?1",
                [&note.id],
            )
            .unwrap();
        assert!(!validate_evidence(&db.connection, &scope, &original).unwrap());
        let current = db.keyword_search(&query()).unwrap().items.remove(0);
        assert!(validate_evidence(&db.connection, &scope, &current).unwrap());
        db.save_note(&note.id, note.revision, "真实证据", "新证据")
            .unwrap();
        assert!(!validate_evidence(&db.connection, &scope, &current).unwrap());
    }
    #[test]
    fn generation_evidence_must_match_active_profile_chunk_and_generation() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Library::open(dir.path()).unwrap();
        db.create_note("证据", "实际证据").unwrap();
        let mut evidence = db.keyword_search(&query()).unwrap().items.remove(0);
        let input = read_index_input(&db.connection, &evidence.target.object_id).unwrap();
        let profile = uuid::Uuid::new_v4().to_string();
        let generation = uuid::Uuid::new_v4().to_string();
        let scope = SearchScope::default();
        evidence.id = "abcdef0123456789".repeat(4);
        evidence.generation_id = Some(generation.clone());
        assert!(!validate_evidence(&db.connection, &scope, &evidence).unwrap());
        db.connection
            .execute(
                "INSERT INTO embedding_profiles VALUES(?1,?2,'now')",
                params![
                    profile,
                    serde_json::to_string(&crate::EmbeddingProfileSpec {
                        endpoint: None,
                        model: "test-profile".into(),
                        revision: "fixture-v1".into(),
                        dimensions: 2,
                        normalized: true,
                        query_prefix: String::new(),
                        passage_prefix: String::new(),
                        tokenizer: "fixture".into(),
                        chunker: "fixture".into(),
                        max_tokens: 16,
                        overlap_tokens: 0,
                        provider: None,
                        output_dimensions: None,
                    })
                    .unwrap()
                ],
            )
            .unwrap();
        db.connection
            .execute(
                "UPDATE search_settings SET active_profile_id=?1",
                [&profile],
            )
            .unwrap();
        db.connection.execute("INSERT INTO index_generations(id,object_id,content_revision,metadata_revision,profile_id,parser_revision,status,chunk_count,created_at,request_hash) VALUES(?1,?2,?3,?4,?5,?6,'ready',1,'now','test')",params![generation,input.object_id,input.content_revision,input.metadata_revision,profile,input.parser_revision]).unwrap();
        db.connection
            .execute(
                "INSERT INTO active_index_generations VALUES(?1,?2,?3)",
                params![input.object_id, profile, generation],
            )
            .unwrap();
        db.connection
            .execute(
                "INSERT INTO index_chunks VALUES(?1,?2,?3,?4,?5,1)",
                params![
                    evidence.id,
                    generation,
                    serde_json::to_string(&evidence.target).unwrap(),
                    evidence.title,
                    evidence.text
                ],
            )
            .unwrap();
        assert!(validate_evidence(&db.connection, &scope, &evidence).unwrap());
        db.connection
            .execute(
                "UPDATE index_chunks SET text='tampered' WHERE generation_id=?1",
                [&generation],
            )
            .unwrap();
        assert!(!validate_evidence(&db.connection, &scope, &evidence).unwrap());
        db.connection
            .execute(
                "UPDATE index_chunks SET text=?1 WHERE generation_id=?2",
                params![evidence.text, generation],
            )
            .unwrap();
        db.connection
            .execute("UPDATE search_settings SET active_profile_id=NULL", [])
            .unwrap();
        assert!(!validate_evidence(&db.connection, &scope, &evidence).unwrap());
        db.connection
            .execute(
                "UPDATE search_settings SET active_profile_id=?1",
                [&profile],
            )
            .unwrap();
        db.connection
            .execute(
                "UPDATE index_generations SET status='failed' WHERE id=?1",
                [&generation],
            )
            .unwrap();
        assert!(!validate_evidence(&db.connection, &scope, &evidence).unwrap());
    }
    #[test]
    fn source_evidence_rejects_stale_parse_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Library::open(dir.path()).unwrap();
        let command = uuid::Uuid::new_v4().to_string();
        let job = db
            .accept_source_import(
                &command,
                crate::ImportSource {
                    input_token: command.clone(),
                    original_name: "证据.txt".into(),
                    format: crate::SourceFormat::Text,
                    origin_url: None,
                    collection_id: None,
                    add_version_to: None,
                    base_row_version: None,
                },
            )
            .unwrap();
        let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
        let bytes = "来源证据".as_bytes();
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
            crate::parse_text_source(&version.id, crate::SourceFormat::Text, bytes).unwrap(),
        )
        .unwrap();
        let evidence = db.keyword_search(&query()).unwrap().items.remove(0);
        let scope = SearchScope::default();
        assert!(validate_evidence(&db.connection, &scope, &evidence).unwrap());
        let mut stale = evidence.clone();
        stale.target.content_revision = uuid::Uuid::new_v4().to_string();
        assert!(!validate_evidence(&db.connection, &scope, &stale).unwrap());
        stale = evidence.clone();
        stale.target.parse_id = Some(uuid::Uuid::new_v4().to_string());
        assert!(!validate_evidence(&db.connection, &scope, &stale).unwrap());
        db.connection
            .execute(
                "DELETE FROM source_active_parses WHERE source_version_id=?1",
                [&version.id],
            )
            .unwrap();
        assert!(!validate_evidence(&db.connection, &scope, &evidence).unwrap());
    }
    #[test]
    fn boilerplate_terms_leave_the_match_but_the_rarest_term_stays() {
        // A template word matches every document; the discriminative term stays.
        assert_eq!(
            discriminative_indexes(&[Some(10_000), Some(10_000), Some(833)]),
            vec![2]
        );
        // Eight times the rarest term is the boundary, and it keeps the term.
        assert_eq!(discriminative_indexes(&[Some(9_000), Some(900)]), vec![1]);
        assert_eq!(
            discriminative_indexes(&[Some(7_200), Some(900)]),
            vec![0, 1]
        );
        // Small libraries keep every term: the floor is 32 documents.
        assert_eq!(discriminative_indexes(&[Some(31), Some(1)]), vec![0, 1]);
        // A single term is never pruned, and unknown frequencies are kept.
        assert_eq!(discriminative_indexes(&[Some(10_000)]), vec![0]);
        assert_eq!(discriminative_indexes(&[None, None]), vec![0, 1]);
        assert_eq!(discriminative_indexes(&[]), Vec::<usize>::new());
    }

    /// A word every document carries must not make the ranked pass score the whole
    /// library, and dropping it must not lose the documents that also carry the
    /// selective word.
    #[test]
    fn a_boilerplate_term_is_pruned_without_losing_selective_hits() {
        let dir = tempfile::tempdir().unwrap();
        let mut library = Library::open(dir.path()).unwrap();
        for index in 0..40 {
            let body = if index % 10 == 0 {
                format!("common 段落 {index} 还带 rare 标记。")
            } else {
                format!("common 段落 {index} 只有公共词。")
            };
            library
                .create_note(&format!("笔记 {index}"), &body)
                .unwrap();
        }
        let request = |text: &str| SearchQuery {
            query: text.into(),
            mode: crate::SearchMode::Lexical,
            scope: SearchScope::default(),
            limit: 8,
            minimum_cosine: None,
        };
        let selective = library.keyword_search(&request("common rare")).unwrap();
        assert_eq!(
            selective.items.len(),
            4,
            "every document carrying the selective term stays reachable"
        );
        let boilerplate = library.keyword_search(&request("common")).unwrap();
        assert_eq!(
            boilerplate.items.len(),
            8,
            "a query that is nothing but boilerplate is not pruned away"
        );
    }
}

/// Diagnostic probes for the P2 retrieval-latency rows. They print cost and
/// quality numbers for the note channel and for the full local search, assert
/// nothing, and are ignored by default:
///
/// ```text
/// cargo test -p pkms-core --release --lib p2_probe -- --ignored --nocapture
/// ```
///
/// with `PKMS_LATENCY_SCALE=1000` or `10000`.
#[cfg(test)]
mod p2_probe {
    use super::*;
    use crate::{Library, SearchMode, SearchQuery, SearchScope, tokens};
    use std::time::Instant;

    const TOPICS: [&str; 12] = [
        "备份", "索引", "权限", "导出", "标签", "回收", "检索", "同步", "批注", "模板", "统计",
        "范围",
    ];

    fn corpus(size: usize) -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let mut library = Library::open(dir.path()).unwrap();
        for index in 0..size {
            let topic = TOPICS[index % TOPICS.len()];
            let body = format!(
                "{topic}记录 {index}：{topic}流程需要保留七天，{topic}失败时按范围重试。\
                 {topic}审批由负责人确认，{topic}结果写入台账并保留版本。其他说明文字用于凑长度。"
            );
            library
                .create_note(&format!("{topic}笔记 {index}"), &body)
                .unwrap();
        }
        (dir, library)
    }

    fn scale() -> usize {
        std::env::var("PKMS_LATENCY_SCALE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1_000)
    }

    fn query() -> SearchQuery {
        SearchQuery {
            query: "备份保留几天？".into(),
            mode: SearchMode::Lexical,
            scope: SearchScope::default(),
            limit: 8,
            minimum_cosine: None,
        }
    }

    fn percentile(samples: &mut [f64], fraction: f64) -> f64 {
        samples.sort_by(f64::total_cmp);
        let index = ((samples.len() as f64 - 1.0) * fraction).ceil() as usize;
        samples[index.min(samples.len() - 1)]
    }

    fn report(label: &str, samples: &[f64], rows: usize) {
        let mut copy = samples.to_vec();
        let p50 = percentile(&mut copy, 0.50);
        let mut copy = samples.to_vec();
        let p95 = percentile(&mut copy, 0.95);
        let max = copy.last().copied().unwrap_or_default();
        println!("shape {label}: p50={p50:.2}ms p95={p95:.2}ms max={max:.2}ms rows={rows}");
    }

    /// The accepted statement: one ranked statement whose single ordering key is
    /// `rank`, so FTS5 itself consumes the order.
    fn ranked(db: &Connection, expression: &str, limit: u32) -> Vec<String> {
        db.prepare(
            "SELECT f.note_id FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=f.note_id \
             WHERE note_fts MATCH ?1 AND rank MATCH 'bm25(0.0,3.0,1.0)' AND n.trashed=0 AND o.trashed=0 \
             AND o.search_enabled=1 AND o.kind='note' ORDER BY rank LIMIT ?2",
        )
        .unwrap()
        .query_map(params![expression, limit], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
    }

    /// The rejected candidate-bound shape, kept so the reason it was rejected
    /// stays reproducible: an unsigned join walk to find the pool bound, then the
    /// same ranked statement fenced by `rowid<=bound`.
    fn bounded(db: &Connection, expression: &str, cap: i64, limit: u32) -> Vec<String> {
        let bound: Option<i64> = db
            .prepare(
                "SELECT max(f.rowid) FROM (SELECT f.rowid AS rowid FROM note_fts f JOIN notes n ON n.id=f.note_id \
                 JOIN objects o ON o.id=f.note_id WHERE note_fts MATCH ?1 AND n.trashed=0 AND o.trashed=0 \
                 AND o.search_enabled=1 AND o.kind='note' LIMIT ?2) f",
            )
            .unwrap()
            .query_row(params![expression, cap], |r| r.get(0))
            .unwrap();
        let Some(bound) = bound else {
            return Vec::new();
        };
        db.prepare(
            "SELECT f.note_id FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=f.note_id \
             WHERE note_fts MATCH ?1 AND f.rowid<=?3 AND rank MATCH 'bm25(0.0,3.0,1.0)' AND n.trashed=0 \
             AND o.trashed=0 AND o.search_enabled=1 AND o.kind='note' ORDER BY rank LIMIT ?2",
        )
        .unwrap()
        .query_map(params![expression, limit, bound], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
    }

    fn plan(db: &Connection, expression: &str, sql: &str, arguments: &[&dyn rusqlite::ToSql]) {
        let mut statement = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let rows = statement
            .query_map(rusqlite::params_from_iter(arguments.iter()), |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        println!("plan[{expression}]: {}", rows.join(" | "));
    }

    #[test]
    #[ignore = "diagnostic probe: prints note-channel shape costs, asserts nothing"]
    fn note_channel_shapes() {
        let size = scale();
        let (_dir, db) = corpus(size);
        let expression = match_expression(&db.connection, &tokens("备份保留几天？")).unwrap();
        let matches: i64 = db
            .connection
            .query_row(
                "SELECT count(*) FROM note_fts WHERE note_fts MATCH ?1",
                [&expression],
                |r| r.get(0),
            )
            .unwrap();
        println!("size={size} expression={expression} matches={matches}");
        plan(
            &db.connection,
            "ranked",
            "SELECT f.note_id FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=f.note_id \
             WHERE note_fts MATCH ?1 AND rank MATCH 'bm25(0.0,3.0,1.0)' AND n.trashed=0 AND o.trashed=0 \
             AND o.search_enabled=1 AND o.kind='note' ORDER BY rank LIMIT ?2",
            &[&expression, &8i64],
        );
        plan(
            &db.connection,
            "pool",
            "SELECT f.rowid FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=f.note_id \
             WHERE note_fts MATCH ?1 AND n.trashed=0 AND o.trashed=0 AND o.search_enabled=1 AND o.kind='note' LIMIT ?2",
            &[&expression, &2048i64],
        );

        let mut pool = Vec::new();
        let mut exact = Vec::new();
        let mut bound = Vec::new();
        let mut count = Vec::new();
        let mut pool_rows = 0usize;
        let mut exact_rows = 0usize;
        let mut overlap = 0usize;
        let mut compared = 0usize;
        for sample in 0..40 {
            let started = Instant::now();
            pool_rows = db
                .connection
                .prepare(
                    "SELECT f.rowid FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=f.note_id \
                     WHERE note_fts MATCH ?1 AND n.trashed=0 AND o.trashed=0 AND o.search_enabled=1 AND o.kind='note' LIMIT ?2",
                )
                .unwrap()
                .query_map(params![expression, 2048i64], |r| r.get::<_, i64>(0))
                .unwrap()
                .count();
            pool.push(started.elapsed().as_secs_f64() * 1000.0);

            let started = Instant::now();
            let reference = ranked(&db.connection, &expression, 8);
            exact_rows = reference.len();
            exact.push(started.elapsed().as_secs_f64() * 1000.0);

            let started = Instant::now();
            let capped = bounded(&db.connection, &expression, 2048, 8);
            bound.push(started.elapsed().as_secs_f64() * 1000.0);

            let started = Instant::now();
            let _ = db
                .connection
                .query_row(
                    "SELECT count(*) FROM note_fts WHERE note_fts MATCH ?1",
                    [&expression],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap();
            count.push(started.elapsed().as_secs_f64() * 1000.0);

            if sample >= 10 {
                compared += reference.len();
                overlap += reference.iter().filter(|id| capped.contains(id)).count();
            }
        }
        pool.sort_by(f64::total_cmp);
        report("pool_scan", &pool, pool_rows);
        report("ranked_exact", &exact, exact_rows);
        report("ranked_capped", &bound, 8);
        report("count_only", &count, matches as usize);
        println!("exact top-8 kept by the capped shape: {overlap}/{compared}");
    }

    #[test]
    #[ignore = "diagnostic probe: prints local search phase timings, asserts nothing"]
    fn keyword_search_phases() {
        let size = scale();
        let (_dir, db) = corpus(size);
        let query = query();
        let terms = tokens(&query.query);
        let mut phases: std::collections::BTreeMap<&str, Vec<f64>> = Default::default();
        let mut total = Vec::new();
        let samples = 120;
        for sample in 0..samples {
            macro_rules! phase {
                ($label:literal, $body:expr) => {{
                    let started = Instant::now();
                    let value = $body;
                    if sample >= 20 {
                        phases
                            .entry($label)
                            .or_default()
                            .push(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    value
                }};
            }
            let expression = phase!(
                "df_counts",
                match_expression(&db.connection, &terms).unwrap()
            );
            let notes = phase!(
                "notes_ranked",
                ranked(&db.connection, &expression, query.limit)
            );
            let sources = phase!(
                "sources_ranked",
                db.search_sources_ranked(&query.query, query.limit, None)
                    .unwrap()
            );
            let annotations = phase!(
                "annotation_inputs",
                db.annotation_search_inputs(&query.scope).unwrap()
            );
            phase!("note_excerpts", {
                for id in &notes {
                    let input = read_index_input(&db.connection, id).unwrap();
                    let block = input.blocks.iter().max_by_key(|b| {
                        let matched = tokens(&b.text);
                        terms
                            .iter()
                            .filter(|term| matched.binary_search(term).is_ok())
                            .count()
                    });
                    if let Some(block) = block {
                        let _ = excerpt(block, &terms);
                    }
                }
            });
            let started = Instant::now();
            let results = db.keyword_search(&query).unwrap();
            if sample >= 20 {
                total.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            assert!(!results.items.is_empty());
            let _ = (sources, annotations);
        }
        let mut copy = total.clone();
        let p50 = percentile(&mut copy, 0.50);
        let mut copy = total.clone();
        let p95 = percentile(&mut copy, 0.95);
        println!(
            "size={size} keyword_search: p50={p50:.2}ms p95={p95:.2}ms max={:.2}ms samples={samples}",
            total.last().copied().unwrap_or_default()
        );
        for (label, samples) in &mut phases {
            let mut copy = samples.clone();
            let p50 = percentile(&mut copy, 0.50);
            let mut copy = samples.clone();
            let p95 = percentile(&mut copy, 0.95);
            println!(
                "phase {label}: p50={p50:.2}ms p95={p95:.2}ms max={:.2}ms",
                copy.last().copied().unwrap_or_default()
            );
        }
    }

    /// The benchmark's exact loop: every local search runs right after the
    /// previous iteration wrote its frozen evidence, so the tail includes page
    /// cache pressure from those writes. This probe repeats that loop with the
    /// shipped page cache and with a larger one.
    #[test]
    #[ignore = "diagnostic probe: prints search/freeze tails, asserts nothing"]
    fn freeze_interleaved_tail() {
        let size = scale();
        let (_dir, mut db) = corpus(size);
        let query = query();
        for (label, pragmas) in [
            ("default_cache", None),
            (
                "cache_64mb_mmap_256mb",
                Some(("PRAGMA cache_size=-65536", "PRAGMA mmap_size=268435456")),
            ),
        ] {
            if let Some((cache, mmap)) = pragmas {
                db.connection.execute_batch(cache).unwrap();
                db.connection.execute_batch(mmap).unwrap();
            }
            let mut search = Vec::new();
            let mut freeze = Vec::new();
            for sample in 0..60 {
                let started = Instant::now();
                let _ = db.keyword_search(&query).unwrap();
                if sample >= 10 {
                    search.push(started.elapsed().as_secs_f64() * 1000.0);
                }
                let accepted = db
                    .accept_answer(
                        &uuid::Uuid::new_v4().to_string(),
                        crate::StartAnswer {
                            max_output_tokens: None,
                            citation_count: Some(5),
                            agent: None,
                            search: query.clone(),
                            model: None,
                        },
                    )
                    .unwrap();
                let lease = db.claim_answer(&accepted.answer.id).unwrap().unwrap();
                let started = Instant::now();
                db.prepare_answer_evidence(&accepted.answer.id, &lease.lease_token, vec![])
                    .unwrap();
                if sample >= 10 {
                    freeze.push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
            let mut copy = search.clone();
            let p50 = percentile(&mut copy, 0.50);
            let mut copy = search.clone();
            let p95 = percentile(&mut copy, 0.95);
            println!(
                "interleaved size={size} {label} search: p50={p50:.2}ms p95={p95:.2}ms max={:.2}ms",
                search.iter().copied().fold(f64::MIN, f64::max)
            );
            let mut copy = freeze.clone();
            let p50 = percentile(&mut copy, 0.50);
            let mut copy = freeze.clone();
            let p95 = percentile(&mut copy, 0.95);
            println!(
                "interleaved size={size} {label} freeze: p50={p50:.2}ms p95={p95:.2}ms max={:.2}ms",
                freeze.iter().copied().fold(f64::MIN, f64::max)
            );
        }
    }

    /// What the rejected candidate bound does to the head of the list once it
    /// actually binds: forty strongly matching notes are appended *after* a
    /// filler corpus, so the exact ranking puts them first while a pool taken in
    /// document order never sees them.
    #[test]
    #[ignore = "diagnostic probe: prints capped-versus-exact overlap, asserts nothing"]
    fn binding_cap_drops_the_head() {
        let size = scale();
        let dir = tempfile::tempdir().unwrap();
        let mut db = Library::open(dir.path()).unwrap();
        for index in 0..size {
            db.create_note(
                &format!("留存笔记 {index}"),
                &format!("保留流程说明 {index}：保留的版本需要定期整理，保留记录用于说明长度。"),
            )
            .unwrap();
        }
        let mut strong = Vec::new();
        for index in 0..40 {
            let note = db
                .create_note(
                    &format!("强烈命中 {index}"),
                    "保留保留保留保留保留保留保留保留 关键结论与保留策略全文。",
                )
                .unwrap();
            strong.push(note.id.clone());
        }
        let expression = "\"保留\"";
        let matches: i64 = db
            .connection
            .query_row(
                "SELECT count(*) FROM note_fts WHERE note_fts MATCH ?1",
                [expression],
                |r| r.get(0),
            )
            .unwrap();
        let exact = ranked(&db.connection, expression, 8);
        let capped = bounded(&db.connection, expression, 2048, 8);
        let mut exact_ms = Vec::new();
        let mut capped_ms = Vec::new();
        let mut plain_ms = Vec::new();
        for sample in 0..30 {
            let started = Instant::now();
            let _ = ranked(&db.connection, expression, 8);
            if sample >= 5 {
                exact_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            let started = Instant::now();
            let _ = bounded(&db.connection, expression, 2048, 8);
            if sample >= 5 {
                capped_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            let started = Instant::now();
            let _ = db
                .connection
                .prepare(
                    "SELECT f.note_id FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=f.note_id \
                     WHERE note_fts MATCH ?1 AND n.trashed=0 AND o.trashed=0 AND o.search_enabled=1 AND o.kind='note' LIMIT 8",
                )
                .unwrap()
                .query_map([expression], |r| r.get::<_, String>(0))
                .unwrap()
                .count();
            if sample >= 5 {
                plain_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        let mut copy = exact_ms.clone();
        let p50 = percentile(&mut copy, 0.50);
        let mut copy = capped_ms.clone();
        let capped_p50 = percentile(&mut copy, 0.50);
        let mut copy = plain_ms.clone();
        let plain_p50 = percentile(&mut copy, 0.50);
        println!(
            "binding_cap cost: exact_ranked_p50={p50:.2}ms capped_p50={capped_p50:.2}ms unranked_first8_p50={plain_p50:.2}ms"
        );
        let exact_strong = exact.iter().filter(|id| strong.contains(id)).count();
        let capped_strong = capped.iter().filter(|id| strong.contains(id)).count();
        println!(
            "binding_cap size={size} matches={matches} exact_top8_strong={exact_strong}/8 capped_top8_strong={capped_strong}/8"
        );
    }
}

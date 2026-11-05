use crate::{
    Error, Library, ParsedSource, Result, SourceBlock, SourceCoverData, SourceFormat,
    SourceMutation, SourcePage, SourceSummary, SourceTextPage, SourceTocItem, SourceVersion,
    SourcesPageRequest, UpdateSourceMetadata, document::valid_uuid, mutations, source_jobs,
};
use crate::{SourceBlockImage, SourceImageData};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_PARSE_BYTES: usize = 32 * 1024 * 1024;

// Cursor consumers and the numbered library browser share exactly one scope.
const SOURCE_SELECTION_SQL: &str = "FROM sources s
    JOIN objects o ON o.id=s.id JOIN source_versions v ON v.id=s.current_version_id
    LEFT JOIN reading_states r ON r.source_id=s.id
    WHERE o.trashed=?1 AND (?2='' OR instr(lower(s.display_title),?2)>0)
    AND (?3 IS NULL OR v.format=?3) AND (?4 IS NULL OR coalesce(r.status,'unread')=?4)
    AND (?5 IS NULL OR o.inbox_state=?5)
    AND (?6 IS NULL OR EXISTS(SELECT 1 FROM memberships m WHERE m.collection_id=?6 AND m.member_id=s.id))
    AND NOT EXISTS(SELECT 1 FROM json_each(?7) wanted WHERE NOT EXISTS(
        SELECT 1 FROM object_tags t WHERE t.object_id=s.id AND t.tag_id=wanted.value))";

fn prepare_source_query(
    filter: Option<crate::SourcesFilter>,
    query: Option<String>,
) -> Result<(crate::SourcesFilter, String)> {
    let mut filter = filter.unwrap_or_default();
    if filter.tag_ids.len() > 100 {
        return Err(Error::InvalidInput("too many source tag filters"));
    }
    for tag in &filter.tag_ids {
        valid_uuid(tag)?;
    }
    filter.tag_ids.sort();
    filter.tag_ids.dedup();
    if let Some(collection) = &filter.collection_id {
        valid_uuid(collection)?;
    }
    let query = query.unwrap_or_default().trim().to_ascii_lowercase();
    if query.chars().count() > 512 || query.chars().any(char::is_control) {
        return Err(Error::InvalidInput("invalid source title query"));
    }
    Ok((filter, query))
}

/// Called inside the metadata/capture transaction; never rewrites per-block FTS rows.
pub(crate) fn index_source_title(db: &Connection, id: &str, title_tokens: &str) -> Result<()> {
    db.execute("DELETE FROM source_title_fts WHERE source_id=?1", [id])?;
    db.execute(
        "INSERT INTO source_title_fts(source_id,title_tokens) VALUES(?1,?2)",
        params![id, title_tokens],
    )?;
    Ok(())
}

fn check_parse_input(
    db: &Connection,
    job_id: &str,
    lease: &str,
    source_id: &str,
    version_id: &str,
    metadata_revision: u32,
) -> Result<SourceSummary> {
    let job = source_jobs::check_lease(db, job_id, lease)?;
    let source = read_source(db, source_id)?;
    if job.source_version_id.as_deref() != Some(version_id)
        || source.current_version_id != version_id
        || (source.metadata_revision != metadata_revision && source.active_parse_id.is_none())
    {
        return Err(Error::InvalidInput("parse source changed"));
    }
    if source.trashed {
        return Err(Error::Trashed);
    }
    Ok(source)
}

pub(crate) fn validate_web_capture(capture: &crate::WebCaptureInput) -> Result<()> {
    source_jobs::validate_web_url(&capture.final_url)?;
    if !(200..300).contains(&capture.http_status)
        || capture.content_type.trim().is_empty()
        || capture.content_type.len() > 1024
        || capture.content_type.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput("invalid webpage response facts"));
    }
    Ok(())
}

pub(crate) fn source_block_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SourceBlock> {
    let geometry: Option<String> = r.get(3)?;
    let pdf_geometry = geometry
        .map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()?;
    Ok(SourceBlock {
        id: r.get(0)?,
        text: r.get(1)?,
        page_index: r.get(2)?,
        pdf_geometry,
    })
}

pub(crate) fn read_source(db: &Connection, id: &str) -> Result<SourceSummary> {
    valid_uuid(id)?;
    let row=db.query_row("SELECT s.display_title,s.current_version_id,o.row_version,s.metadata_revision,o.trashed,v.format,a.parse_id,s.updated_at,EXISTS(SELECT 1 FROM source_blocks b WHERE b.parse_id=a.parse_id AND length(b.text)>0),v.origin_url,v.original_name,v.blob_hash,(SELECT substr(b.text,1,240) FROM source_blocks b WHERE b.parse_id=a.parse_id AND length(trim(b.text))>0 ORDER BY b.position LIMIT 1),s.author FROM sources s JOIN objects o ON o.id=s.id JOIN source_versions v ON v.id=s.current_version_id LEFT JOIN source_active_parses a ON a.source_version_id=v.id WHERE s.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get::<_,String>(5)?,r.get::<_,Option<String>>(6)?,r.get(7)?,r.get::<_,bool>(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get::<_,Option<String>>(13)?))).optional()?.ok_or(Error::NotFound)?;
    Ok(SourceSummary {
        id: id.into(),
        display_title: row.0,
        current_version_id: row.1,
        row_version: row.2,
        metadata_revision: row.3,
        trashed: row.4,
        format: serde_json::from_value(serde_json::Value::String(row.5))?,
        text_ready: row.8,
        lexical_ready: row.8,
        active_parse_id: row.6,
        updated_at: row.7,
        origin_url: row.9,
        original_name: row.10,
        content_hash: row.11,
        preview_text: row.12,
        author: row.13,
    })
}
pub(crate) fn read_version(db: &Connection, id: &str) -> Result<SourceVersion> {
    valid_uuid(id)?;
    let schema: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let web_capture = if schema >= 10 {
        db.query_row("SELECT final_url,http_status,content_type,captured_at FROM source_web_captures WHERE source_version_id=?1", [id], |r| Ok(crate::WebCapture { final_url:r.get(0)?,http_status:r.get(1)?,content_type:r.get(2)?,captured_at:r.get(3)? })).optional()?
    } else {
        None
    };
    let row=db.query_row("SELECT v.source_id,v.blob_hash,v.original_name,v.origin_url,v.captured_at,v.format,b.size FROM source_versions v JOIN blobs b ON b.hash=v.blob_hash WHERE v.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get::<_,String>(5)?,r.get::<_,i64>(6)?))).optional()?.ok_or(Error::NotFound)?;
    Ok(SourceVersion {
        web_capture,
        id: id.into(),
        source_id: row.0,
        blob_hash: row.1,
        original_name: row.2,
        origin_url: row.3,
        captured_at: row.4,
        format: serde_json::from_value(serde_json::Value::String(row.5))?,
        size: u64::try_from(row.6).map_err(|_| Error::IntegrityFailure)?,
    })
}
pub(crate) fn verify_source_blob(path: &Path, hash: &str, size: u64) -> Result<()> {
    if size == 0 || size > MAX_SOURCE_BYTES {
        return Err(Error::IntegrityFailure);
    }
    let mut file = File::open(path)?;
    if file.metadata()?.len() != size {
        return Err(Error::IntegrityFailure);
    }
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    if format!("{:x}", digest.finalize()) != hash {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}
impl Library {
    /// Fresh restore staging only: bounded chunks; tokenization occurs before each write transaction.
    pub(crate) fn rebuild_source_search(
        &mut self,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        self.connection.execute("DELETE FROM source_fts", [])?;
        self.connection
            .execute("DELETE FROM source_title_fts", [])?;
        let mut title_after = String::new();
        loop {
            check("rebuild_source_titles")?;
            let rows = self
                .connection
                .prepare("SELECT id,display_title FROM sources WHERE id>?1 ORDER BY id LIMIT 100")?
                .query_map([&title_after], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if rows.is_empty() {
                break;
            }
            let prepared = rows
                .into_iter()
                .map(|(id, title)| (id, crate::tokenize(&title)))
                .collect::<Vec<_>>();
            let tx = self.connection.transaction()?;
            for (id, title) in prepared {
                index_source_title(&tx, &id, &title)?;
                title_after = id;
            }
            tx.commit()?;
        }
        let mut after = String::new();
        loop {
            check("rebuild_source_search")?;
            let rows=self.connection.prepare("SELECT s.id,s.current_version_id,b.id,s.display_title,b.text FROM sources s JOIN source_active_parses a ON a.source_version_id=s.current_version_id JOIN source_blocks b ON b.parse_id=a.parse_id WHERE b.id>?1 ORDER BY b.id LIMIT 100")?.query_map([&after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
            if rows.is_empty() {
                break;
            }
            let prepared = rows
                .into_iter()
                .map(|(source, version, block, title, text)| {
                    (
                        source,
                        version,
                        block,
                        {
                            let _ = title;
                            String::new()
                        },
                        crate::tokenize(&text),
                    )
                })
                .collect::<Vec<_>>();
            let tx = self.connection.transaction()?;
            for (source, version, block, title, text) in prepared {
                tx.execute("INSERT INTO source_fts(source_id,source_version_id,block_id,title_tokens,body_tokens) VALUES(?1,?2,?3,?4,?5)",params![source,version,block,title,text])?;
                after = block;
            }
            tx.commit()?;
        }
        Ok(())
    }
    pub fn source_versions(
        &self,
        source_id: &str,
        after_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SourceVersion>> {
        self.source(source_id)?;
        crate::collections::validate_limit(limit)?;
        if let Some(id) = after_id {
            valid_uuid(id)?;
        }
        let after = after_id
            .map(|id| {
                self.connection
                    .query_row(
                        "SELECT captured_at FROM source_versions WHERE source_id=?1 AND id=?2",
                        params![source_id, id],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?
                    .ok_or(Error::InvalidInput(
                        "version cursor belongs to another source",
                    ))
            })
            .transpose()?;
        let ids=self.connection.prepare("SELECT id FROM source_versions WHERE source_id=?1 AND (?2 IS NULL OR captured_at<?2 OR (captured_at=?2 AND id<?3)) ORDER BY captured_at DESC,id DESC LIMIT ?4")?.query_map(params![source_id,after,after_id,limit],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        ids.iter().map(|id| self.source_version(id)).collect()
    }
    pub fn search_sources(&self, query: &str, limit: u32) -> Result<Vec<crate::SourceSearchHit>> {
        self.search_sources_scoped(query, limit, None)
    }
    pub(crate) fn search_sources_scoped(
        &self,
        query: &str,
        limit: u32,
        eligible: Option<&[String]>,
    ) -> Result<Vec<crate::SourceSearchHit>> {
        crate::collections::validate_limit(limit)?;
        if query.len() > 4096 {
            return Err(Error::InvalidInput("search query exceeds bounds"));
        }
        let tokens = crate::tokens(query);
        if tokens.is_empty() {
            return Ok(vec![]);
        }
        let terms = tokens
            .iter()
            .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
            .collect::<Vec<_>>();
        let title_expression = terms.join(" AND ");
        let title_candidates = terms.join(" OR ");
        // A title-only match represents the source once, at its first active block.
        // Compute remaining terms per title, then share each remaining body MATCH
        // across sources with the same title terms. Do not probe FTS once per block.
        // Title-only hits rank first; body hits use BM25 for their required body terms.
        // Every lookup belongs to one read snapshot, including active pointers.
        let sql = "WITH title_candidates AS MATERIALIZED (
            SELECT rowid,source_id,bm25(source_title_fts,0.0,3.0) AS score
            FROM source_title_fts WHERE source_title_fts MATCH ?2
        ), title_conditions AS MATERIALIZED (
            SELECT t.source_id,t.score,(
                SELECT group_concat('body_tokens : ' || term.value,' AND ')
                FROM json_each(?3) term WHERE NOT EXISTS (
                    SELECT 1 FROM source_title_fts match_title
                    WHERE match_title.rowid=t.rowid AND source_title_fts MATCH term.value
                )
            ) AS body_query FROM title_candidates t
        ), searches AS MATERIALIZED (
            SELECT 'body_tokens : (' || ?1 || ')' AS body_query,0 AS mixed
            UNION ALL
            SELECT DISTINCT body_query,1 FROM title_conditions WHERE body_query IS NOT NULL
        ), hits AS (
            SELECT s.id AS source_id,s.current_version_id AS version_id,b.id AS block_id,
                   s.display_title AS title,b.text,b.page_index,t.score,0 AS tier
            FROM title_conditions t JOIN sources s ON s.id=t.source_id
            JOIN objects o ON o.id=s.id
            JOIN source_active_parses a ON a.source_version_id=s.current_version_id
            JOIN source_blocks b ON b.id=(SELECT first.id FROM source_blocks first
                WHERE first.parse_id=a.parse_id ORDER BY first.position LIMIT 1)
            WHERE t.body_query IS NULL AND o.trashed=0 AND o.search_enabled=1
            UNION ALL
            SELECT s.id,s.current_version_id,b.id,s.display_title,b.text,b.page_index,
                   bm25(source_fts,0.0,0.0,0.0,0.0,1.0),1
            FROM searches q CROSS JOIN source_fts f
            JOIN sources s ON s.id=f.source_id AND s.current_version_id=f.source_version_id
            JOIN objects o ON o.id=s.id
            JOIN source_active_parses a ON a.source_version_id=s.current_version_id
            JOIN source_blocks b ON b.id=f.block_id AND b.parse_id=a.parse_id
            WHERE source_fts MATCH q.body_query AND o.trashed=0 AND o.search_enabled=1
              AND ((q.mixed=0 AND s.id NOT IN (SELECT source_id FROM title_candidates))
                OR (q.mixed=1 AND s.id IN (SELECT source_id FROM title_conditions
                    WHERE body_query=q.body_query)))
        ) SELECT source_id,version_id,block_id,title,text,page_index,score
          FROM hits WHERE (?5 IS NULL OR source_id IN (SELECT value FROM json_each(?5))) ORDER BY tier,score,source_id,block_id LIMIT ?4";
        Ok(self
            .connection
            .prepare(sql)?
            .query_map(
                params![
                    title_expression,
                    title_candidates,
                    serde_json::to_string(&terms)?,
                    limit,
                    eligible.map(serde_json::to_string).transpose()?
                ],
                |r| {
                    Ok(crate::SourceSearchHit {
                        source_id: r.get(0)?,
                        source_version_id: r.get(1)?,
                        block_id: r.get(2)?,
                        title: r.get(3)?,
                        text: r.get(4)?,
                        page_index: r.get(5)?,
                        bm25: r.get(6)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    /// Natural-language retrieval: distinct term coverage first, then title/body RRF.
    /// Keep this separate from the source browser's exact AND search contract.
    pub(crate) fn search_sources_ranked(
        &self,
        query: &str,
        limit: u32,
        eligible: Option<&[String]>,
    ) -> Result<Vec<crate::SourceSearchHit>> {
        crate::collections::validate_limit(limit)?;
        if query.len() > 4096 {
            return Err(Error::InvalidInput("search query exceeds bounds"));
        }
        if eligible.is_some_and(|ids| ids.is_empty()) {
            return Ok(vec![]);
        }
        let terms = crate::tokens(query);
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let quoted_terms = terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>();
        let expression = quoted_terms.join(" OR ");
        // A template word repeated by every document costs one body match per
        // block without separating anything, which is why the notes channel has
        // pruned those terms since V18-G02; the same rule now decides the body
        // match here. The title channel keeps the full expression: a title match
        // is one row per source, and a title-only hit must not disappear because
        // its other words are common. Dropped terms still count for coverage.
        let body_expression = crate::retrieval::source_match_expression(&self.connection, &terms)?;
        // Only identifiers/scores enter the ranking tables. Each query term scans
        // FTS postings once per channel; never re-run MATCH for every candidate.
        // Count overlapping title/body terms once. Load text only for the final
        // limited result; all rankings and locator reads share one snapshot.
        let sql = "WITH body_matches AS MATERIALIZED (
            SELECT s.id AS source_id,s.current_version_id AS version_id,b.id AS block_id,
                   f.rowid AS fts_rowid,
                   bm25(source_fts,0.0,0.0,0.0,0.0,1.0) AS score
            FROM source_fts f
            JOIN sources s ON s.id=f.source_id AND s.current_version_id=f.source_version_id
            JOIN objects o ON o.id=s.id
            JOIN source_active_parses a ON a.source_version_id=s.current_version_id
            JOIN source_blocks b ON b.id=f.block_id AND b.parse_id=a.parse_id
            WHERE source_fts MATCH ('body_tokens : (' || ?1 || ')')
              AND o.trashed=0 AND o.search_enabled=1
              AND (?2 IS NULL OR s.id IN (SELECT value FROM json_each(?2)))
        ), title_matches AS MATERIALIZED (
            SELECT s.id AS source_id,s.current_version_id AS version_id,a.parse_id,
                   f.rowid AS fts_rowid,
                   bm25(source_title_fts,0.0,3.0) AS score
            FROM source_title_fts f JOIN sources s ON s.id=f.source_id
            JOIN objects o ON o.id=s.id
            JOIN source_active_parses a ON a.source_version_id=s.current_version_id
            WHERE source_title_fts MATCH ?5 AND o.trashed=0 AND o.search_enabled=1
              AND (?2 IS NULL OR s.id IN (SELECT value FROM json_each(?2)))
              AND EXISTS(SELECT 1 FROM source_blocks b WHERE b.parse_id=a.parse_id)
        ), title_terms AS MATERIALIZED (
            SELECT DISTINCT t.source_id,term.key AS term_id
            FROM json_each(?4) term CROSS JOIN source_title_fts f
            JOIN title_matches t ON t.fts_rowid=f.rowid
            WHERE source_title_fts MATCH term.value
        ), title_coverage AS (
            SELECT source_id,count(*) AS covered FROM title_terms GROUP BY source_id
        ), body_coverage AS (
            SELECT b.block_id,count(DISTINCT term.key) AS covered
            FROM json_each(?4) term CROSS JOIN source_fts f
            JOIN body_matches b ON b.fts_rowid=f.rowid
            LEFT JOIN title_terms t ON t.source_id=b.source_id AND t.term_id=term.key
            WHERE source_fts MATCH ('body_tokens : ' || term.value) AND t.term_id IS NULL
            GROUP BY b.block_id
        ), ranked_body AS (
            SELECT *,row_number() OVER (ORDER BY score,source_id,block_id) AS rank
            FROM body_matches
        ), ranked_title AS MATERIALIZED (
            SELECT *,row_number() OVER (ORDER BY score,source_id) AS rank
            FROM title_matches
        ), candidates AS (
            SELECT b.source_id,b.version_id,b.block_id,b.score,
                   1.0/(60+b.rank)+coalesce(1.0/(60+t.rank),0.0) AS fusion_score
            FROM ranked_body b LEFT JOIN ranked_title t ON t.source_id=b.source_id
            UNION ALL
            SELECT t.source_id,t.version_id,
                   (SELECT b.id FROM source_blocks b WHERE b.parse_id=t.parse_id
                    ORDER BY b.position LIMIT 1),t.score,1.0/(60+t.rank)
            FROM ranked_title t WHERE NOT EXISTS(
                SELECT 1 FROM body_matches b WHERE b.source_id=t.source_id)
        ), selected AS MATERIALIZED (
            SELECT c.*,coalesce(b.covered,0)+coalesce(t.covered,0) AS coverage
            FROM candidates c LEFT JOIN body_coverage b ON b.block_id=c.block_id
            LEFT JOIN title_coverage t ON t.source_id=c.source_id
            ORDER BY coverage DESC,fusion_score DESC,c.source_id,c.block_id LIMIT ?3
        ) SELECT selected.source_id,selected.version_id,selected.block_id,s.display_title,
                 b.text,b.page_index,selected.score
          FROM selected JOIN sources s ON s.id=selected.source_id
          JOIN source_blocks b ON b.id=selected.block_id
          ORDER BY selected.coverage DESC,selected.fusion_score DESC,selected.source_id,selected.block_id";
        Ok(self
            .connection
            .prepare(sql)?
            .query_map(
                params![
                    body_expression,
                    eligible.map(serde_json::to_string).transpose()?,
                    limit,
                    serde_json::to_string(&quoted_terms)?,
                    expression
                ],
                |row| {
                    Ok(crate::SourceSearchHit {
                        source_id: row.get(0)?,
                        source_version_id: row.get(1)?,
                        block_id: row.get(2)?,
                        title: row.get(3)?,
                        text: row.get(4)?,
                        page_index: row.get(5)?,
                        bm25: row.get(6)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn prepare_source_export(
        &self,
        version_id: &str,
        destination: impl AsRef<Path>,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<crate::PreparedOutput<crate::SourceExportReport>> {
        let (version, mut file) = self.source_original_file(version_id)?;
        let destination = destination.as_ref();
        let mut output = crate::portable::new_output(destination)?;
        let mut writer = zip::ZipWriter::new(output.as_file_mut());
        let extension = match version.format {
            SourceFormat::Text => "txt",
            SourceFormat::Markdown => "md",
            SourceFormat::Html => "html",
            SourceFormat::Pdf => "pdf",
            SourceFormat::Docx => "docx",
            SourceFormat::Doc => "doc",
            SourceFormat::Epub => "epub",
            SourceFormat::Mobi => "mobi",
            SourceFormat::Csv => "csv",
            SourceFormat::Xlsx => "xlsx",
            SourceFormat::Xls => "xls",
            SourceFormat::Pptx => "pptx",
        };
        let original_path = format!("original/{version_id}.{extension}");
        writer
            .start_file(
                &original_path,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .map_err(crate::portable::archive_error)?;
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut size = 0u64;
        loop {
            check("export_original")?;
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            if size > version.size {
                return Err(Error::IntegrityFailure);
            }
            digest.update(&buffer[..n]);
            writer.write_all(&buffer[..n])?;
        }
        if size != version.size || format!("{:x}", digest.finalize()) != version.blob_hash {
            return Err(Error::IntegrityFailure);
        }
        crate::portable::zip_bytes(
            &mut writer,
            "source-version.json",
            &serde_json::to_vec_pretty(&version)?,
        )?;
        let report=crate::SourceExportReport{source_id:version.source_id.clone(),source_version_id:version.id,original_path,warnings:vec!["导出包含所选不可变原件；完整历史、摘录、阅读状态和主题请使用完整备份。HTML 原件可能包含外部网站脚本，应用内安全阅读使用提取文本。".into()]};
        crate::portable::zip_bytes(
            &mut writer,
            "export-report.json",
            &serde_json::to_vec_pretty(&report)?,
        )?;
        writer.finish().map_err(crate::portable::archive_error)?;
        Ok(crate::PreparedOutput {
            file: output,
            destination: destination.to_owned(),
            report,
        })
    }
    pub fn source(&self, id: &str) -> Result<SourceSummary> {
        read_source(&self.connection, id)
    }
    /// The source that already owns this content hash, if the library has one.
    /// The directory scan reports it as a duplicate before importing; the import
    /// path itself deduplicates the same content into the same source.
    pub fn source_for_content_hash(&self, hash: &str) -> Result<Option<SourceSummary>> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT source_id FROM source_file_dedup WHERE blob_hash=?1",
                [hash],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|id| read_source(&self.connection, &id)).transpose()
    }
    pub fn source_version(&self, id: &str) -> Result<SourceVersion> {
        read_version(&self.connection, id)
    }
    /// Native adapters that need to hand an immutable original to a local
    /// renderer receive its verified path, never an arbitrary IPC path.
    pub fn source_original_path(&self, version_id: &str) -> Result<(SourceVersion, PathBuf)> {
        let version = self.source_version(version_id)?;
        let path = self.blob_path(&version.blob_hash, false)?;
        if std::fs::metadata(&path)?.len() != version.size {
            return Err(Error::IntegrityFailure);
        }
        Ok((version, path))
    }

    /// Native resource adapter receives an opened file, never an arbitrary path in IPC.
    pub fn source_original_file(&self, version_id: &str) -> Result<(SourceVersion, File)> {
        let (version, path) = self.source_original_path(version_id)?;
        let file = File::open(path)?;
        Ok((version, file))
    }
    pub fn capture_source_original(
        &mut self,
        job_id: &str,
        lease: &str,
        input: &mut dyn Read,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<SourceVersion> {
        self.capture_original_inner(job_id, lease, input, None, check)
    }
    pub fn capture_webpage_original(
        &mut self,
        job_id: &str,
        lease: &str,
        input: &mut dyn Read,
        capture: crate::WebCaptureInput,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<SourceVersion> {
        validate_web_capture(&capture)?;
        self.capture_original_inner(job_id, lease, input, Some(capture), check)
    }
    fn capture_original_inner(
        &mut self,
        job_id: &str,
        lease: &str,
        input: &mut dyn Read,
        capture: Option<crate::WebCaptureInput>,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<SourceVersion> {
        let job = source_jobs::check_lease(&self.connection, job_id, lease)?;
        if capture.is_some()
            && (job.input.format != SourceFormat::Html || job.input.origin_url.is_none())
        {
            return Err(Error::InvalidInput("web capture requires a webpage import"));
        }
        if let Some(version) = job.source_version_id {
            return self.source_version(&version);
        }
        let workspace = self.database_path.parent().ok_or(Error::IntegrityFailure)?;
        let mut temp = tempfile::Builder::new()
            .prefix("source-capture-")
            .suffix(".partial")
            .tempfile_in(workspace)?;
        let mut digest = Sha256::new();
        let mut bytes = [0u8; 64 * 1024];
        let mut size = 0u64;
        loop {
            check("capture_original")?;
            source_jobs::check_lease(&self.connection, job_id, lease)?;
            let n = input.read(&mut bytes)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            if size > MAX_SOURCE_BYTES {
                return Err(Error::InvalidInput("source exceeds 256 MiB"));
            }
            digest.update(&bytes[..n]);
            temp.write_all(&bytes[..n])?;
            #[cfg(test)]
            fault_tests::checkpoint("staging_write", &self.connection, temp.path(), size, None);
        }
        if size == 0 {
            return Err(Error::InvalidInput("source is empty"));
        }
        temp.as_file_mut().seek(SeekFrom::Start(0))?;
        if job.input.format == SourceFormat::Pdf {
            let mut header = [0; 5];
            temp.read_exact(&mut header)?;
            if &header != b"%PDF-" {
                return Err(Error::InvalidInput("source is not a PDF"));
            }
        }
        let hash = format!("{:x}", digest.finalize());
        temp.as_file().sync_all()?;
        #[cfg(test)]
        fault_tests::checkpoint(
            "after_hash",
            &self.connection,
            temp.path(),
            size,
            Some(&hash),
        );
        let path = self.blob_path(&hash, true)?;
        match temp.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                verify_source_blob(&path, &hash, size)?
            }
            Err(error) => return Err(error.error.into()),
        }
        #[cfg(unix)]
        File::open(path.parent().ok_or(Error::IntegrityFailure)?)?.sync_all()?;
        #[cfg(test)]
        fault_tests::checkpoint("after_blob", &self.connection, &path, size, Some(&hash));
        check("register_original")?;
        let initial_title_tokens = crate::tokenize(job.input.original_name.trim());
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = source_jobs::check_lease(&tx, job_id, lease)?;
        if let Some(version) = job.source_version_id {
            return read_version(&tx, &version);
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO blobs(hash,size,media_type,width,height,created_at) VALUES(?1,?2,?3,NULL,NULL,?4) ON CONFLICT(hash) DO NOTHING",params![hash,size as i64,job.input.format.media_type(),now])?;
        let existing = if job.input.add_version_to.is_none() && job.input.origin_url.is_none() {
            tx.query_row(
                "SELECT source_id FROM source_file_dedup WHERE blob_hash=?1",
                [&hash],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        } else {
            None
        };
        let deduplicated = existing.is_some();
        let (source_id, version_id) = if let Some(existing) = existing {
            let version:String=tx.query_row("SELECT id FROM source_versions WHERE source_id=?1 AND blob_hash=?2 ORDER BY captured_at,id LIMIT 1",params![existing,hash],|r|r.get(0))?;
            (existing, version)
        } else {
            let source_id = job
                .input
                .add_version_to
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let version_id = uuid::Uuid::new_v4().to_string();
            if job.input.add_version_to.is_some() {
                let source = read_source(&tx, &source_id)?;
                mutations::check_version(
                    &source_id,
                    job.input.base_row_version.unwrap_or(0),
                    source.row_version,
                )?;
                if source.trashed {
                    return Err(Error::Trashed);
                }
                tx.execute(
                    "UPDATE sources SET current_version_id=?2,updated_at=?3 WHERE id=?1",
                    params![source_id, version_id, now],
                )?;
                tx.execute(
                    "UPDATE objects SET row_version=row_version+1 WHERE id=?1",
                    [&source_id],
                )?;
                // Current-version/active-parse joins invalidate old derived hits.
                // Reclaim their FTS rows separately instead of holding this mutation
                // transaction while deleting an entire large document.
            } else {
                tx.execute(
                    "INSERT INTO objects(id,kind,inbox_state) VALUES(?1,'source','pending')",
                    [&source_id],
                )?;
                tx.execute("INSERT INTO sources(id,display_title,current_version_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",params![source_id,job.input.original_name.trim(),version_id,now])?;
                index_source_title(&tx, &source_id, &initial_title_tokens)?;
            }
            tx.execute("INSERT INTO source_versions(id,source_id,blob_hash,original_name,origin_url,format,captured_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![version_id,source_id,hash,job.input.original_name,job.input.origin_url,serde_json::to_value(job.input.format)?.as_str(),now])?;
            if let Some(capture) = &capture {
                tx.execute("INSERT INTO source_web_captures(source_version_id,final_url,http_status,content_type,captured_at) VALUES(?1,?2,?3,?4,?5)",params![version_id,capture.final_url,capture.http_status,capture.content_type,now])?;
            }
            if job.input.origin_url.is_none() {
                tx.execute("INSERT INTO source_file_dedup(blob_hash,source_id) VALUES(?1,?2) ON CONFLICT(blob_hash) DO NOTHING",params![hash,source_id])?;
            }
            (source_id, version_id)
        };
        let source = read_source(&tx, &source_id)?;
        let organized = job.input.collection_id.is_some() || !job.tag_names.is_empty();
        let tagged = if source.trashed {
            false
        } else {
            crate::organization::assign_tag_names(&tx, &source_id, &job.tag_names)?
        };
        if let Some(collection) = job.input.collection_id
            && !source.trashed
        {
            let trashed: bool = tx.query_row(
                "SELECT trashed FROM objects WHERE id=?1",
                [&collection],
                |r| r.get(0),
            )?;
            if trashed {
                return Err(Error::Trashed);
            }
            let added=tx.execute("INSERT INTO memberships(collection_id,member_id,position) SELECT ?1,?2,coalesce(max(position),-1)+1 FROM memberships WHERE collection_id=?1 ON CONFLICT(collection_id,member_id) DO NOTHING",params![collection,source_id])?;
            if added > 0 {
                tx.execute(
                    "UPDATE objects SET row_version=row_version+1 WHERE id=?1",
                    [&collection],
                )?;
                tx.execute(
                    "UPDATE collections SET updated_at=?2 WHERE id=?1",
                    params![collection, now],
                )?;
            }
        }
        if organized && !source.trashed {
            let current: Option<String> = tx.query_row(
                "SELECT inbox_state FROM objects WHERE id=?1",
                [&source_id],
                |row| row.get(0),
            )?;
            if tagged || current.as_deref() != Some("processed") {
                tx.execute(
                    "UPDATE objects SET row_version=row_version+1,inbox_state='processed' WHERE id=?1",
                    [&source_id],
                )?;
            }
        }
        tx.execute("UPDATE source_jobs SET source_id=?2,source_version_id=?3,stage='original_saved',updated_at=?4 WHERE id=?1",params![job_id,source_id,version_id,now])?;
        let finished_stage = if !deduplicated {
            None
        } else if source.trashed {
            Some("existing_trashed_source")
        } else if source.current_version_id != version_id {
            Some("existing_historical_version")
        } else if source.active_parse_id.is_some() {
            Some("existing_source")
        } else {
            None
        };
        if let Some(stage) = finished_stage {
            tx.execute("UPDATE source_jobs SET status='succeeded',stage=?2,lease_token=NULL,lease_expires_at=NULL,error_code=NULL WHERE id=?1",params![job_id,stage])?;
            tx.execute(
                "UPDATE source_job_attempts SET finished_at=?3 WHERE job_id=?1 AND lease_token=?2",
                params![job_id, lease, now],
            )?;
        }
        source_jobs::event(&tx, job_id)?;
        let version = read_version(&tx, &version_id)?;
        tx.commit()?;
        Ok(version)
    }
    pub fn publish_source_parse(
        &mut self,
        job_id: &str,
        lease: &str,
        parsed: ParsedSource,
    ) -> Result<SourceSummary> {
        validate_parse(&parsed)?;
        let initial = source_jobs::check_lease(&self.connection, job_id, lease)?;
        if initial.source_version_id.as_deref() != Some(&parsed.source_version_id) {
            return Err(Error::InvalidInput("parse input version mismatch"));
        }
        let version = self.source_version(&parsed.source_version_id)?;
        validate_parse_format(&parsed, version.format)?;
        // Cleaning shapes the derived text only. The original stays byte for byte,
        // and the report travels with the parse so the reader can show what it did.
        let mut parsed = parsed;
        let cover_blob = if let Some(cover) = parsed.cover.as_ref() {
            let bytes = STANDARD
                .decode(cover.data_base64.as_bytes())
                .map_err(|_| Error::InvalidInput("invalid_cover"))?;
            if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 {
                return Err(Error::InvalidInput("cover_too_large"));
            }
            Some(self.store_image_bytes(&bytes)?)
        } else {
            None
        };
        let mut image_blobs = Vec::new();
        for image in &parsed.images {
            let bytes = STANDARD
                .decode(image.data_base64.as_bytes())
                .map_err(|_| Error::InvalidInput("invalid_source_image"))?;
            if bytes.is_empty() || bytes.len() > 12 * 1024 * 1024 {
                return Err(Error::InvalidInput("source_image_too_large"));
            }
            let info = self.store_image_bytes(&bytes)?;
            if info.media_type != image.media_type {
                return Err(Error::InvalidInput("source_image_type_mismatch"));
            }
            image_blobs.push((
                image.block_position as usize,
                info.hash,
                info.media_type,
                image.alt.clone(),
            ));
        }
        let toc_json = serde_json::to_string(&parsed.toc)?;
        let cleaning = crate::cleaning::clean_blocks(
            &mut parsed.blocks,
            &crate::cleaning::cleaning_settings(&self.connection)?.params,
        );
        let initial_source = self.source(&version.source_id)?;
        let parsed_title = if matches!(
            version.format,
            SourceFormat::Html | SourceFormat::Epub | SourceFormat::Mobi
        ) && initial_source.metadata_revision == 1
        {
            parsed
                .title
                .as_deref()
                .filter(|title| !title.trim().is_empty())
                .map(mutations::validate_name)
                .transpose()?
        } else {
            None
        };
        // The file's own metadata supplies the author once. It never overwrites a
        // value the library already shows, and a format without one stays None so
        // the interface can say "not provided" instead of inventing a name.
        let parsed_author = parsed
            .author
            .as_deref()
            .map(str::trim)
            .filter(|author| !author.is_empty())
            .map(|author| author.chars().take(300).collect::<String>());
        let title_tokens = crate::tokenize(
            parsed_title
                .as_deref()
                .unwrap_or(&initial_source.display_title),
        );
        let tokens = parsed
            .blocks
            .iter()
            .map(|b| crate::tokenize(&b.text))
            .collect::<Vec<_>>();
        let geometry = parsed
            .blocks
            .iter()
            .map(|block| {
                block
                    .pdf_geometry
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = source_jobs::check_lease(&tx, job_id, lease)?;
        let source = read_source(&tx, &version.source_id)?;
        if job.source_version_id.as_deref() != Some(&parsed.source_version_id)
            || source.current_version_id != parsed.source_version_id
            || (source.metadata_revision != initial_source.metadata_revision
                && source.active_parse_id.is_none())
        {
            return Err(Error::InvalidInput("parse source changed"));
        }
        if source.trashed {
            return Err(Error::Trashed);
        }
        // Two imports may capture the same original before either parser finishes.
        // The first published parse owns stable block IDs for this immutable version.
        if source.active_parse_id.is_some() {
            let now = Utc::now().to_rfc3339();
            tx.execute("UPDATE source_jobs SET status='succeeded',stage='existing_source',lease_token=NULL,lease_expires_at=NULL,error_code=NULL,updated_at=?2 WHERE id=?1",params![job_id,now])?;
            tx.execute(
                "UPDATE source_job_attempts SET finished_at=?3 WHERE job_id=?1 AND lease_token=?2",
                params![job_id, lease, now],
            )?;
            source_jobs::event(&tx, job_id)?;
            tx.commit()?;
            return Ok(source);
        }
        let parse_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO source_parses(id,source_version_id,parser_version,page_count,needs_ocr,created_at,cleaning_json,toc_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![parse_id,parsed.source_version_id,parsed.parser_version,parsed.page_count,parsed.needs_ocr,now,serde_json::to_string(&cleaning)?,toc_json])?;
        tx.execute(
            "INSERT INTO source_parse_staging(parse_id,job_id,lease_token) VALUES(?1,?2,?3)",
            params![parse_id, job_id, lease],
        )?;
        tx.commit()?;
        // Inactive blocks and their derived FTS rows stay invisible until the final
        // pointer transaction. Yield the SQLite writer between bounded batches.
        let mut start = 0;
        while start < parsed.blocks.len() {
            let mut end = start;
            let mut batch_bytes = 0;
            while end < parsed.blocks.len() && end - start < 100 {
                let bytes = parsed.blocks[end].text.len()
                    + tokens[end].len()
                    + geometry[end].as_ref().map_or(0, String::len)
                    + title_tokens.len();
                if end > start && batch_bytes + bytes > 256 * 1024 {
                    break;
                }
                batch_bytes += bytes;
                end += 1;
            }
            let blocks = &parsed.blocks[start..end];
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            check_parse_input(
                &tx,
                job_id,
                lease,
                &source.id,
                &parsed.source_version_id,
                source.metadata_revision,
            )?;
            let mut insert_block = tx.prepare("INSERT INTO source_blocks(id,parse_id,position,text,page_index,geometry_json) VALUES(?1,?2,?3,?4,?5,?6)")?;
            let mut insert_fts = tx.prepare("INSERT INTO source_fts(source_id,source_version_id,block_id,title_tokens,body_tokens) VALUES(?1,?2,?3,?4,?5)")?;
            for (offset, block) in blocks.iter().enumerate() {
                let position = start + offset;
                insert_block.execute(params![
                    block.id,
                    parse_id,
                    position as u32,
                    block.text,
                    block.page_index,
                    geometry[position]
                ])?;
                insert_fts.execute(params![
                    source.id,
                    parsed.source_version_id,
                    block.id,
                    "",
                    tokens[position]
                ])?;
            }
            drop(insert_block);
            drop(insert_fts);
            tx.commit()?;
            start = end;
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = check_parse_input(
            &tx,
            job_id,
            lease,
            &source.id,
            &parsed.source_version_id,
            source.metadata_revision,
        )?;
        // Another ordinary import may win while our inactive batches were written.
        // Preserve that first complete parse and leave this stage invisible for GC.
        if current.active_parse_id.is_some() {
            let now = Utc::now().to_rfc3339();
            tx.execute("UPDATE source_jobs SET status='succeeded',stage='existing_source',lease_token=NULL,lease_expires_at=NULL,error_code=NULL,updated_at=?2 WHERE id=?1",params![job_id,now])?;
            tx.execute(
                "UPDATE source_job_attempts SET finished_at=?3 WHERE job_id=?1 AND lease_token=?2",
                params![job_id, lease, now],
            )?;
            source_jobs::event(&tx, job_id)?;
            tx.commit()?;
            return Ok(current);
        }
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "DELETE FROM source_parse_staging WHERE parse_id=?1 AND job_id=?2 AND lease_token=?3",
            params![parse_id, job_id, lease],
        )?;
        if let Some(cover) = cover_blob {
            tx.execute(
                "INSERT INTO source_covers(parse_id,blob_hash,created_at) VALUES(?1,?2,?3)",
                params![parse_id, cover.hash, now],
            )?;
        }
        for (position, hash, media_type, alt) in &image_blobs {
            let block = parsed
                .blocks
                .get(*position)
                .ok_or(Error::IntegrityFailure)?;
            tx.execute(
                "INSERT INTO source_assets(parse_id,blob_hash,media_type,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(parse_id,blob_hash) DO NOTHING",
                params![parse_id, hash, media_type, now],
            )?;
            tx.execute(
                "INSERT INTO source_block_images(parse_id,block_id,blob_hash,position,alt_text) VALUES(?1,?2,?3,?4,?5)",
                params![parse_id, block.id, hash, *position as u32, alt],
            )?;
        }
        tx.execute("INSERT INTO source_active_parses(source_version_id,parse_id) VALUES(?1,?2) ON CONFLICT(source_version_id) DO UPDATE SET parse_id=excluded.parse_id",params![parsed.source_version_id,parse_id])?;
        tx.execute(
            "UPDATE sources SET updated_at=?2 WHERE id=?1",
            params![source.id, now],
        )?;
        if let Some(author) = parsed_author
            && source.author.is_none()
        {
            tx.execute(
                "UPDATE sources SET author=?2 WHERE id=?1",
                params![source.id, author],
            )?;
        }
        if let Some(title) = parsed_title {
            tx.execute("UPDATE sources SET display_title=?2,metadata_revision=metadata_revision+1 WHERE id=?1",params![source.id,title])?;
            index_source_title(&tx, &source.id, &title_tokens)?;
        }
        // Active parse identity is part of search/index input even without a title change.
        tx.execute(
            "UPDATE objects SET row_version=row_version+1 WHERE id=?1",
            [&source.id],
        )?;
        tx.execute("UPDATE source_jobs SET status='succeeded',stage=CASE WHEN ?2 THEN 'needs_ocr' ELSE 'lexical_ready' END,lease_token=NULL,lease_expires_at=NULL,error_code=NULL,updated_at=?3 WHERE id=?1",params![job_id,parsed.needs_ocr,now])?;
        tx.execute(
            "UPDATE source_job_attempts SET finished_at=?3 WHERE job_id=?1 AND lease_token=?2",
            params![job_id, lease, now],
        )?;
        source_jobs::event(&tx, job_id)?;
        let source = read_source(&tx, &source.id)?;
        tx.commit()?;
        Ok(source)
    }
    pub fn source_text_page(
        &self,
        version_id: &str,
        parse_id: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> Result<SourceTextPage> {
        valid_uuid(version_id)?;
        crate::collections::validate_limit(limit)?;
        let snapshot = self.connection.unchecked_transaction()?;
        let selected = match parse_id {
            Some(id) => {
                valid_uuid(id)?;
                id.to_owned()
            }
            None => snapshot
                .query_row(
                    "SELECT parse_id FROM source_active_parses WHERE source_version_id=?1",
                    [version_id],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(Error::NotFound)?,
        };
        let (page_count,needs_ocr,cleaning,parser_version,toc_json)=snapshot.query_row("SELECT page_count,needs_ocr,cleaning_json,parser_version,toc_json FROM source_parses WHERE id=?1 AND source_version_id=?2 AND NOT EXISTS(SELECT 1 FROM source_parse_staging WHERE parse_id=?1)",params![selected,version_id],|r|Ok((r.get(0)?,r.get(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?))).optional()?.ok_or(Error::NotFound)?;
        let cleaning = cleaning
            .map(|json| serde_json::from_str(&json))
            .transpose()?;
        let toc: Vec<SourceTocItem> = toc_json
            .map(|json| serde_json::from_str(&json))
            .transpose()?
            .unwrap_or_default();
        let has_cover: bool = snapshot.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_covers WHERE parse_id=?1)",
            [&selected],
            |r| r.get(0),
        )?;
        let mut blocks=snapshot.prepare("SELECT id,text,page_index,geometry_json FROM source_blocks WHERE parse_id=?1 AND position>=?2 ORDER BY position LIMIT ?3")?.query_map(params![selected,offset,limit+1],source_block_from_row)?.collect::<std::result::Result<Vec<_>,_>>()?;
        let next_offset = if blocks.len() > limit as usize {
            blocks.truncate(limit as usize);
            Some(
                offset
                    .checked_add(limit)
                    .ok_or(Error::InvalidInput("source offset overflow"))?,
            )
        } else {
            None
        };
        let end_offset = offset.saturating_add(blocks.len() as u32);
        let images = snapshot
            .prepare(
                "SELECT i.block_id,i.blob_hash,a.media_type,i.alt_text FROM source_block_images i JOIN source_assets a ON a.parse_id=i.parse_id AND a.blob_hash=i.blob_hash WHERE i.parse_id=?1 AND i.position>=?2 AND i.position<?3 ORDER BY i.position",
            )?
            .query_map(params![selected, offset, end_offset], |row| {
                Ok(SourceBlockImage {
                    block_id: row.get(0)?,
                    blob_hash: row.get(1)?,
                    media_type: row.get(2)?,
                    alt: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let tables = crate::source_tables::extract(&blocks);
        Ok(SourceTextPage {
            source_version_id: version_id.into(),
            parse_id: selected,
            blocks,
            page_count,
            cleaning,
            needs_ocr,
            toc,
            has_cover,
            images,
            ocr_applied: parser_version.contains("+ocr-"),
            ocr_engine: parser_version
                .contains("+ocr-tesseract5")
                .then(|| "tesseract5".to_owned()),
            tables,
            next_offset,
        })
    }
    pub fn source_cover(
        &self,
        version_id: &str,
        parse_id: Option<&str>,
    ) -> Result<SourceCoverData> {
        valid_uuid(version_id)?;
        let selected = match parse_id {
            Some(id) => {
                valid_uuid(id)?;
                id.to_owned()
            }
            None => self
                .connection
                .query_row(
                    "SELECT parse_id FROM source_active_parses WHERE source_version_id=?1",
                    [version_id],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(Error::NotFound)?,
        };
        let hash: String = self
            .connection
            .query_row(
                "SELECT c.blob_hash FROM source_covers c JOIN source_parses p ON p.id=c.parse_id WHERE p.id=?1 AND p.source_version_id=?2 AND NOT EXISTS(SELECT 1 FROM source_parse_staging WHERE parse_id=p.id)",
                params![selected, version_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let (info, bytes) = self.image_blob(&hash)?;
        Ok(SourceCoverData {
            source_version_id: version_id.into(),
            parse_id: selected,
            media_type: info.media_type,
            data_base64: STANDARD.encode(bytes),
        })
    }

    pub fn source_asset(
        &self,
        version_id: &str,
        parse_id: &str,
        blob_hash: &str,
    ) -> Result<SourceImageData> {
        valid_uuid(version_id)?;
        valid_uuid(parse_id)?;
        if !crate::portable::valid_hash(blob_hash) {
            return Err(Error::InvalidInput("invalid source image hash"));
        }
        let media_type: String = self
            .connection
            .query_row(
                "SELECT a.media_type FROM source_assets a JOIN source_parses p ON p.id=a.parse_id WHERE a.parse_id=?1 AND a.blob_hash=?2 AND p.source_version_id=?3 AND NOT EXISTS(SELECT 1 FROM source_parse_staging WHERE parse_id=p.id)",
                params![parse_id, blob_hash, version_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let (info, bytes) = self.image_blob(blob_hash)?;
        if info.media_type != media_type {
            return Err(Error::IntegrityFailure);
        }
        Ok(SourceImageData {
            source_version_id: version_id.into(),
            parse_id: parse_id.into(),
            blob_hash: blob_hash.into(),
            media_type,
            data_base64: STANDARD.encode(bytes),
        })
    }
    pub fn sources_page(&self, request: SourcesPageRequest) -> Result<SourcePage> {
        crate::collections::validate_limit(request.limit)?;
        let (filter, query) = prepare_source_query(request.filter, request.query)?;
        let format = filter.format.map(serde_json::to_value).transpose()?;
        let reading = filter
            .reading_status
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        let tag_ids = serde_json::to_string(&filter.tag_ids)?;
        let snapshot = self.connection.unchecked_transaction()?;
        let generation: i64 =
            snapshot.query_row("SELECT generation FROM source_list_state", [], |r| r.get(0))?;
        let workspace: String = snapshot.query_row(
            "SELECT workspace_id FROM workspace_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let scope = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                1,
                self.database_path.to_string_lossy(),
                workspace,
                &filter
            ))?)
        );
        let cursor: Option<(String, String, i64, bool, String, String)> =
            crate::collections::decode_cursor(request.cursor.as_deref())?;
        if let Some((updated, id, g, trashed, q, cursor_scope)) = &cursor {
            valid_uuid(id)?;
            chrono::DateTime::parse_from_rfc3339(updated)
                .map_err(|_| Error::InvalidInput("invalid source cursor timestamp"))?;
            if *trashed != request.trashed || q != &query || cursor_scope != &scope {
                return Err(Error::InvalidInput("source cursor does not match query"));
            }
            if *g != generation {
                return Err(Error::QueryExpired);
            }
        }
        let mut ids = snapshot
            .prepare(&format!(
                "SELECT s.id {SOURCE_SELECTION_SQL}
            AND (?8 IS NULL OR s.updated_at<?8 OR (s.updated_at=?8 AND s.id>?9))
            ORDER BY s.updated_at DESC,s.id LIMIT ?10"
            ))?
            .query_map(
                params![
                    request.trashed,
                    query,
                    format.as_ref().and_then(|v| v.as_str()),
                    reading.as_ref().and_then(|v| v.as_str()),
                    filter.inbox_state.map(|v| v.as_str()),
                    filter.collection_id,
                    tag_ids,
                    cursor.as_ref().map(|v| &v.0),
                    cursor.as_ref().map(|v| &v.1),
                    request.limit + 1
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = ids.len() > request.limit as usize;
        ids.truncate(request.limit as usize);
        let items = ids
            .iter()
            .map(|id| read_source(&snapshot, id))
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            let last = items.last().ok_or(Error::IntegrityFailure)?;
            Some(serde_json::to_string(&(
                &last.updated_at,
                &last.id,
                generation,
                request.trashed,
                query,
                scope,
            ))?)
        } else {
            None
        };
        Ok(SourcePage { items, next_cursor })
    }
    /// Count and rows come from one read snapshot and one filter predicate.
    pub fn sources_browse(
        &self,
        request: crate::SourcesBrowseRequest,
    ) -> Result<crate::SourcesBrowsePage> {
        crate::collections::validate_limit(request.limit)?;
        if request.page == 0 {
            return Err(Error::InvalidInput("invalid source page"));
        }
        let (filter, query) = prepare_source_query(request.filter, request.query)?;
        let reading = filter
            .reading_status
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        let tag_ids = serde_json::to_string(&filter.tag_ids)?;
        let snapshot = self.connection.unchecked_transaction()?;
        let total: u32 = snapshot.query_row(
            &format!("SELECT count(*) {SOURCE_SELECTION_SQL}"),
            params![
                request.trashed,
                query,
                filter.format.map(SourceFormat::stored),
                reading.as_ref().and_then(|v| v.as_str()),
                filter.inbox_state.map(|v| v.as_str()),
                filter.collection_id,
                tag_ids
            ],
            |r| r.get(0),
        )?;
        let page = request.page.min(total.max(1).div_ceil(request.limit));
        let ids = snapshot
            .prepare(&format!(
                "SELECT s.id {SOURCE_SELECTION_SQL}
            ORDER BY s.updated_at DESC,s.id LIMIT ?8 OFFSET ?9"
            ))?
            .query_map(
                params![
                    request.trashed,
                    query,
                    filter.format.map(SourceFormat::stored),
                    reading.as_ref().and_then(|v| v.as_str()),
                    filter.inbox_state.map(|v| v.as_str()),
                    filter.collection_id,
                    tag_ids,
                    request.limit,
                    i64::from(page - 1) * i64::from(request.limit)
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let items = ids
            .iter()
            .map(|id| read_source(&snapshot, id))
            .collect::<Result<Vec<_>>>()?;
        Ok(crate::SourcesBrowsePage {
            items,
            total,
            page,
            limit: request.limit,
        })
    }
    pub fn update_source_metadata(
        &mut self,
        command_id: &str,
        mut request: UpdateSourceMetadata,
    ) -> Result<SourceMutation> {
        request.display_title = mutations::validate_name(&request.display_title)?;
        let hash = mutations::request_hash(command_id, "sources.update_metadata", &request)?;
        let title_tokens = crate::tokenize(&request.display_title);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = mutations::replay(&tx, command_id, &hash, "sources.update_metadata")? {
            return Ok(old);
        }
        let source = read_source(&tx, &request.source_id)?;
        mutations::check_version(&source.id, request.base_row_version, source.row_version)?;
        if source.trashed {
            return Err(Error::Trashed);
        }
        tx.execute("UPDATE sources SET display_title=?2,metadata_revision=metadata_revision+1,updated_at=?3 WHERE id=?1",params![source.id,request.display_title,Utc::now().to_rfc3339()])?;
        tx.execute(
            "UPDATE objects SET row_version=row_version+1 WHERE id=?1",
            [&source.id],
        )?;
        index_source_title(&tx, &source.id, &title_tokens)?;
        let result = SourceMutation {
            source: Some(read_source(&tx, &source.id)?),
            annotation: None,
            reading: None,
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        mutations::store_receipt(
            &tx,
            &hash,
            "sources.update_metadata",
            &source.id,
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
}
pub(crate) fn validate_parse(parsed: &ParsedSource) -> Result<()> {
    valid_uuid(&parsed.source_version_id)?;
    if parsed.parser_version.is_empty()
        || parsed.parser_version.len() > 100
        || parsed.blocks.len() > 100_000
        || parsed.page_count == Some(0)
    {
        return Err(Error::InvalidInput("invalid parse metadata"));
    }
    if parsed.author.as_deref().is_some_and(|author| {
        author.trim().is_empty()
            || author.chars().count() > 300
            || author.chars().any(char::is_control)
    }) {
        return Err(Error::InvalidInput("invalid parse author"));
    }
    let mut ids = BTreeSet::new();
    let mut size = 0usize;
    for block in &parsed.blocks {
        valid_uuid(&block.id)?;
        size += block.text.len();
        if let Some(geometry) = &block.pdf_geometry {
            let crop = &geometry.crop_box;
            if block.page_index.is_none()
                || geometry.coordinate_space != "pdf_user_space"
                || !matches!(geometry.rotation, 0 | 90 | 180 | 270)
                || crop.iter().any(|v| !v.is_finite())
                || crop[2] <= crop[0]
                || crop[3] <= crop[1]
                || geometry.chars.len() != block.text.chars().count()
            {
                return Err(Error::InvalidInput("invalid PDF block geometry"));
            }
            for (index, ch) in geometry.chars.iter().enumerate() {
                if ch.start_codepoint != index as u32 || ch.end_codepoint != index as u32 + 1 {
                    return Err(Error::InvalidInput(
                        "PDF character offsets must use Unicode codepoints",
                    ));
                }
                if let Some(rect) = ch.rect
                    && (rect.iter().any(|v| !v.is_finite())
                        || rect[2] < rect[0]
                        || rect[3] < rect[1])
                {
                    return Err(Error::InvalidInput("invalid PDF character rectangle"));
                }
            }
            size += serde_json::to_vec(geometry)?.len();
        }
        if !ids.insert(&block.id)
            || block.text.len() > 64 * 1024
            || block.text.contains('\0')
            || block
                .page_index
                .is_some_and(|p| parsed.page_count.is_none_or(|count| p >= count))
        {
            return Err(Error::InvalidInput("invalid parse block"));
        }
    }
    if parsed.toc.len() > 5_000 {
        return Err(Error::InvalidInput("invalid source table of contents"));
    }
    for item in &parsed.toc {
        let title = item.title.trim();
        if title.is_empty()
            || title.chars().count() > 300
            || title.chars().any(char::is_control)
            || !(1..=16).contains(&item.level)
        {
            return Err(Error::InvalidInput("invalid source table of contents"));
        }
        if let Some(position) = item.block_position
            && position as usize >= parsed.blocks.len()
        {
            return Err(Error::InvalidInput("invalid source table of contents"));
        }
        if let Some(block_id) = item.block_id.as_deref() {
            valid_uuid(block_id)?;
            if !parsed.blocks.iter().any(|block| block.id == block_id) {
                return Err(Error::InvalidInput("invalid source table of contents"));
            }
        }
    }
    if let Some(cover) = &parsed.cover {
        if cover.data_base64.is_empty()
            || cover.data_base64.len() > 6 * 1024 * 1024
            || cover.media_type.is_empty()
            || cover.media_type.len() > 100
            || cover.media_type.chars().any(char::is_control)
        {
            return Err(Error::InvalidInput("invalid cover"));
        }
        size += cover.data_base64.len();
    }
    if parsed.images.len() > 256 {
        return Err(Error::InvalidInput("too many source images"));
    }
    let mut image_positions = BTreeSet::new();
    let mut image_encoded_bytes = 0usize;
    for image in &parsed.images {
        if image.block_position as usize >= parsed.blocks.len()
            || !image_positions.insert(image.block_position)
            || !matches!(
                image.media_type.as_str(),
                "image/png" | "image/jpeg" | "image/webp"
            )
            || image.data_base64.is_empty()
            || image.data_base64.len() > 16 * 1024 * 1024
            || image.alt.chars().count() > 300
            || image.alt.chars().any(char::is_control)
        {
            return Err(Error::InvalidInput("invalid source image"));
        }
        image_encoded_bytes = image_encoded_bytes
            .checked_add(image.data_base64.len())
            .ok_or(Error::InvalidInput("parsed source exceeds 32 MiB"))?;
    }
    size = size
        .checked_add(image_encoded_bytes)
        .ok_or(Error::InvalidInput("parsed source exceeds 32 MiB"))?;
    let toc_size = serde_json::to_vec(&parsed.toc)?.len();
    if size > MAX_PARSE_BYTES || toc_size > 1024 * 1024 {
        return Err(Error::InvalidInput("parsed source exceeds 32 MiB"));
    }
    Ok(())
}

pub(crate) fn validate_parse_format(parsed: &ParsedSource, format: SourceFormat) -> Result<()> {
    let valid = match format {
        SourceFormat::Pdf => {
            parsed.page_count.is_some() && parsed.blocks.iter().all(|b| b.page_index.is_some())
        }
        // A deck's page count is its real slide count. Slides are not PDF page
        // indexes, so every extracted block still carries no page geometry.
        SourceFormat::Pptx => {
            parsed.page_count.is_some()
                && !parsed.needs_ocr
                && parsed
                    .blocks
                    .iter()
                    .all(|b| b.page_index.is_none() && b.pdf_geometry.is_none())
        }
        _ => {
            parsed.page_count.is_none()
                && !parsed.needs_ocr
                && parsed
                    .blocks
                    .iter()
                    .all(|b| b.page_index.is_none() && b.pdf_geometry.is_none())
        }
    };
    if !valid {
        return Err(Error::InvalidInput(
            "parse output does not match source format",
        ));
    }
    Ok(())
}
/// Bounded UTF-8 text parsing outside database transactions. Original bytes stay immutable.
pub fn parse_text_source(
    source_version_id: &str,
    format: SourceFormat,
    bytes: &[u8],
) -> Result<ParsedSource> {
    valid_uuid(source_version_id)?;
    if !matches!(format, SourceFormat::Text | SourceFormat::Markdown) {
        return Err(Error::InvalidInput("format requires its dedicated parser"));
    }
    if bytes.len() > MAX_PARSE_BYTES {
        return Err(Error::InvalidInput(
            "text source exceeds 32 MiB parse limit",
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::InvalidInput("text source must use UTF-8"))?
        .trim_start_matches('\u{feff}');
    if text.contains('\0') {
        return Err(Error::InvalidInput("text source contains NUL"));
    }
    let mut blocks = vec![];
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let mut chunk = String::new();
        for ch in line.chars() {
            if chunk.len() + ch.len_utf8() > 16 * 1024 {
                if blocks.len() >= 100_000 {
                    return Err(Error::InvalidInput("source exceeds 100000 blocks"));
                }
                blocks.push(SourceBlock {
                    id: uuid::Uuid::new_v4().to_string(),
                    text: std::mem::take(&mut chunk),
                    page_index: None,
                    pdf_geometry: None,
                });
            }
            chunk.push(ch);
        }
        if !chunk.is_empty() {
            if blocks.len() >= 100_000 {
                return Err(Error::InvalidInput("source exceeds 100000 blocks"));
            }
            blocks.push(SourceBlock {
                id: uuid::Uuid::new_v4().to_string(),
                text: chunk,
                page_index: None,
                pdf_geometry: None,
            });
        }
    }
    let parsed = ParsedSource {
        title: None,
        source_version_id: source_version_id.into(),
        parser_version: "utf8-lines-v1".into(),
        author: None,
        blocks,
        page_count: None,
        needs_ocr: false,
        toc: Vec::new(),
        cover: None,
        images: Vec::new(),
    };
    validate_parse(&parsed)?;
    Ok(parsed)
}

#[cfg(test)]
#[path = "sources_fault_tests.rs"]
mod fault_tests;

//! Validation of knowledge relationships, including every historical revision.
use crate::{BackupCounts, BackupFile, Error, NodeType, Result, notes};
use rusqlite::{Connection, OpenFlags, params};
use std::{collections::BTreeSet, io::Read, path::Path};

pub(crate) fn validate_image(db: &Connection, path: &Path, hash: &str) -> Result<()> {
    let expected = db.query_row(
        "SELECT size,media_type,width,height FROM blobs WHERE hash=?1",
        [hash],
        |r| {
            Ok(crate::BlobInfo {
                hash: hash.into(),
                size: r.get(0)?,
                media_type: r.get(1)?,
                width: r.get(2)?,
                height: r.get(3)?,
            })
        },
    )?;
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(20 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 20 * 1024 * 1024
        || crate::portable::hash_bytes(&bytes) != hash
        || crate::attachments::decode_image(&bytes, hash.into())? != expected
    {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}

pub(crate) fn validate_blob(db: &Connection, path: &Path, hash: &str) -> Result<()> {
    let schema: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let is_image: bool = db.query_row(
        if schema >= 34 { "SELECT EXISTS(SELECT 1 FROM note_revision_blobs WHERE blob_hash=?1) OR EXISTS(SELECT 1 FROM proposal_blobs WHERE blob_hash=?1) OR EXISTS(SELECT 1 FROM source_covers WHERE blob_hash=?1)" } else if schema >= 12 { "SELECT EXISTS(SELECT 1 FROM note_revision_blobs WHERE blob_hash=?1) OR EXISTS(SELECT 1 FROM proposal_blobs WHERE blob_hash=?1)" } else { "SELECT EXISTS(SELECT 1 FROM note_revision_blobs WHERE blob_hash=?1)" },
        [hash],
        |r| r.get(0),
    )?;
    if is_image {
        validate_image(db, path, hash)?;
    } else {
        let size: i64 =
            db.query_row("SELECT size FROM blobs WHERE hash=?1", [hash], |r| r.get(0))?;
        crate::sources::verify_source_blob(
            path,
            hash,
            u64::try_from(size).map_err(|_| Error::IntegrityFailure)?,
        )?;
    }
    Ok(())
}

pub(crate) fn read_only(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.pragma_update(None, "trusted_schema", false)?;
    db.pragma_update(None, "query_only", true)?;
    Ok(db)
}

pub(crate) fn inspect_database(db: &Connection) -> Result<(String, BackupCounts, Vec<BackupFile>)> {
    let schema: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let app: i64 = db.pragma_query_value(None, "application_id", |r| r.get(0))?;
    if !(7..=crate::migrations::CURRENT_SCHEMA_VERSION).contains(&schema) || app != 1_347_112_275 {
        return Err(Error::UnsupportedDatabase);
    }
    crate::migrations::validate_history_at(db, schema)?;
    let workspace_id: String = db.query_row(
        "SELECT workspace_id FROM workspace_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let count = |table: &str| -> Result<u32> {
        Ok(db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
    };
    let counts = BackupCounts {
        review_entries: if schema >= 13 {
            count("review_entries")?
        } else {
            0
        },
        review_events: if schema >= 13 {
            count("review_events")?
        } else {
            0
        },
        index_rebuild_jobs: if schema >= 12 {
            count("index_rebuild_jobs")?
        } else {
            0
        },
        embedding_profiles: if schema >= 12 {
            count("embedding_profiles")?
        } else {
            0
        },
        index_generations: if schema >= 12 {
            count("index_generations")?
        } else {
            0
        },
        index_chunks: if schema >= 12 {
            count("index_chunks")?
        } else {
            0
        },
        answers: if schema >= 12 { count("answers")? } else { 0 },
        answer_feedback: if schema >= 12 {
            count("answer_feedback")?
        } else {
            0
        },
        proposals: if schema >= 12 { count("proposals")? } else { 0 },
        web_captures: if schema >= 10 {
            count("source_web_captures")?
        } else {
            0
        },
        sources: if schema >= 9 { count("sources")? } else { 0 },
        source_versions: if schema >= 9 {
            count("source_versions")?
        } else {
            0
        },
        annotations: if schema >= 9 {
            count("annotations")?
        } else {
            0
        },
        source_jobs: if schema >= 9 {
            count("source_jobs")?
        } else {
            0
        },
        source_covers: if schema >= 34 {
            count("source_covers")?
        } else {
            0
        },
        source_assets: if schema >= 35 {
            count("source_assets")?
        } else {
            0
        },
        source_block_images: if schema >= 35 {
            count("source_block_images")?
        } else {
            0
        },
        voice_captures: if schema >= 39 {
            count("voice_captures")?
        } else {
            0
        },
        portable_jobs: if schema >= 8 {
            count("portable_jobs")?
        } else {
            0
        },
        notes: count("notes")?,
        revisions: count("note_revisions")?,
        objects: count("objects")?,
        collections: count("collections")?,
        memberships: count("memberships")?,
        tags: count("tags")?,
        tag_assignments: count("object_tags")?,
        saved_views: count("saved_views")?,
        canvases: if schema >= 25 { count("canvases")? } else { 0 },
        manual_links: count("note_links")?,
        body_links: count("body_links")?,
        revision_blob_references: count("note_revision_blobs")?,
    };
    let blob_sql = if schema >= 39 {
        "SELECT b.hash,b.size FROM blobs b WHERE EXISTS(SELECT 1 FROM note_revision_blobs r WHERE r.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_versions v WHERE v.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM proposal_blobs p WHERE p.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_covers c WHERE c.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_assets a WHERE a.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM voice_captures vc WHERE vc.audio_blob_hash=b.hash) ORDER BY b.hash"
    } else if schema >= 35 {
        "SELECT b.hash,b.size FROM blobs b WHERE EXISTS(SELECT 1 FROM note_revision_blobs r WHERE r.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_versions v WHERE v.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM proposal_blobs p WHERE p.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_covers c WHERE c.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_assets a WHERE a.blob_hash=b.hash) ORDER BY b.hash"
    } else if schema >= 34 {
        "SELECT b.hash,b.size FROM blobs b WHERE EXISTS(SELECT 1 FROM note_revision_blobs r WHERE r.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_versions v WHERE v.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM proposal_blobs p WHERE p.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_covers c WHERE c.blob_hash=b.hash) ORDER BY b.hash"
    } else if schema >= 12 {
        "SELECT b.hash,b.size FROM blobs b WHERE EXISTS(SELECT 1 FROM note_revision_blobs r WHERE r.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_versions v WHERE v.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM proposal_blobs p WHERE p.blob_hash=b.hash) ORDER BY b.hash"
    } else if schema >= 9 {
        "SELECT b.hash,b.size FROM blobs b WHERE EXISTS(SELECT 1 FROM note_revision_blobs r WHERE r.blob_hash=b.hash) OR EXISTS(SELECT 1 FROM source_versions v WHERE v.blob_hash=b.hash) ORDER BY b.hash"
    } else {
        "SELECT DISTINCT b.hash,b.size FROM blobs b JOIN note_revision_blobs r ON r.blob_hash=b.hash ORDER BY b.hash"
    };
    let mut stmt = db.prepare(blob_sql)?;
    let blobs = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .map(|row| {
            let (hash, size) = row?;
            let size = u64::try_from(size).map_err(|_| Error::IntegrityFailure)?;
            if !crate::portable::valid_hash(&hash)
                || size == 0
                || size
                    > if schema >= 9 {
                        crate::sources::MAX_SOURCE_BYTES
                    } else {
                        20 * 1024 * 1024
                    }
            {
                return Err(Error::IntegrityFailure);
            }
            Ok(BackupFile {
                path: format!("blobs/sha256/{}/{}", &hash[..2], hash),
                sha256: hash,
                size,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((workspace_id, counts, blobs))
}

fn require_empty(db: &Connection, sql: &str) -> Result<()> {
    if db.prepare(sql)?.exists([])? {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}

fn validate_sources(db: &Connection, check: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
    let schema: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if schema >= 10 {
        for sql in [
            "SELECT 1 FROM source_active_parses a JOIN source_parse_staging s ON s.parse_id=a.parse_id",
            "SELECT 1 FROM source_parse_staging s JOIN source_parses p ON p.id=s.parse_id JOIN source_jobs j ON j.id=s.job_id WHERE p.source_version_id IS NOT j.source_version_id",
            "SELECT 1 FROM annotations a JOIN source_blocks b ON b.id=json_extract(a.locator_json,'$.block_id') JOIN source_parse_staging s ON s.parse_id=b.parse_id WHERE json_extract(a.locator_json,'$.kind')='text'",
            "SELECT 1 FROM reading_states r JOIN source_blocks b ON b.id=json_extract(r.locator_json,'$.block_id') JOIN source_parse_staging s ON s.parse_id=b.parse_id WHERE json_extract(r.locator_json,'$.kind')='text'",
        ] {
            require_empty(db, sql)?;
        }
        let stages = db
            .prepare("SELECT parse_id,job_id,lease_token FROM source_parse_staging")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (parse, job, lease) in stages {
            crate::document::valid_uuid(&parse)?;
            crate::document::valid_uuid(&job)?;
            crate::document::valid_uuid(&lease)?;
        }
    }
    for sql in [
        "SELECT 1 FROM source_blocks WHERE length(CAST(text AS BLOB))>65536 OR instr(text,char(0))>0 OR length(CAST(geometry_json AS BLOB))>33554432",
        "SELECT 1 FROM source_blocks GROUP BY parse_id HAVING count(*)>100000 OR sum(length(CAST(text AS BLOB))+coalesce(length(CAST(geometry_json AS BLOB)),0))>33554432",
        "SELECT 1 FROM annotations WHERE length(CAST(locator_json AS BLOB))>131072 OR length(CAST(comment AS BLOB))>65536",
        "SELECT 1 FROM reading_states WHERE length(CAST(locator_json AS BLOB))>131072",
        "SELECT 1 FROM source_jobs WHERE length(CAST(input_json AS BLOB))>16384",
        "SELECT 1 FROM sources s LEFT JOIN objects o ON o.id=s.id LEFT JOIN source_versions v ON v.id=s.current_version_id WHERE o.kind IS NOT 'source' OR v.source_id IS NOT s.id",
        "SELECT 1 FROM annotations a LEFT JOIN objects o ON o.id=a.id WHERE o.kind IS NOT 'annotation'",
        "SELECT 1 FROM objects o WHERE (o.kind='source' AND NOT EXISTS(SELECT 1 FROM sources s WHERE s.id=o.id)) OR (o.kind='annotation' AND NOT EXISTS(SELECT 1 FROM annotations a WHERE a.id=o.id))",
        "SELECT 1 FROM source_active_parses a JOIN source_parses p ON p.id=a.parse_id WHERE a.source_version_id!=p.source_version_id",
        "SELECT 1 FROM reading_states r JOIN source_versions v ON v.id=r.source_version_id WHERE r.source_id!=v.source_id",
        "SELECT 1 FROM source_file_dedup d WHERE NOT EXISTS(SELECT 1 FROM source_versions v WHERE v.source_id=d.source_id AND v.blob_hash=d.blob_hash)",
        "SELECT 1 FROM source_jobs j JOIN source_versions v ON v.id=j.source_version_id WHERE j.source_id IS NOT v.source_id",
        "SELECT 1 FROM source_jobs WHERE (source_id IS NULL)!=(source_version_id IS NULL) OR (status='running' AND (lease_token IS NULL OR lease_expires_at IS NULL))",
    ] {
        require_empty(db, sql)?;
    }
    if schema >= 34 {
        require_empty(
            db,
            "SELECT 1 FROM source_covers c LEFT JOIN source_parses p ON p.id=c.parse_id WHERE p.id IS NULL",
        )?;
        require_empty(
            db,
            "SELECT 1 FROM source_parses WHERE length(CAST(toc_json AS BLOB))>1048576",
        )?;
    }
    if schema >= 35 {
        require_empty(
            db,
            "SELECT 1 FROM source_assets a LEFT JOIN source_parses p ON p.id=a.parse_id LEFT JOIN blobs b ON b.hash=a.blob_hash WHERE p.id IS NULL OR b.hash IS NULL OR a.media_type NOT IN ('image/png','image/jpeg','image/webp')",
        )?;
        require_empty(
            db,
            "SELECT 1 FROM source_block_images i LEFT JOIN source_blocks b ON b.id=i.block_id AND b.parse_id=i.parse_id LEFT JOIN source_assets a ON a.parse_id=i.parse_id AND a.blob_hash=i.blob_hash WHERE b.id IS NULL OR a.blob_hash IS NULL OR i.position!=b.position OR length(CAST(i.alt_text AS BLOB))>1200",
        )?;
        require_empty(
            db,
            "SELECT 1 FROM source_block_images GROUP BY parse_id HAVING count(*)>256",
        )?;
    }
    let parses = db
        .prepare(
            if schema >= 10 { "SELECT id,source_version_id,parser_version,page_count,needs_ocr FROM source_parses WHERE id NOT IN (SELECT parse_id FROM source_parse_staging)" } else { "SELECT id,source_version_id,parser_version,page_count,needs_ocr FROM source_parses" },
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get(3)?,
                r.get(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, source_version_id, parser_version, page_count, needs_ocr) in parses {
        check("verify_source_text")?;
        crate::document::valid_uuid(&id)?;
        let blocks = db
            .prepare(
                "SELECT id,text,page_index,geometry_json FROM source_blocks WHERE parse_id=?1 ORDER BY position",
            )?
            .query_map([&id], crate::sources::source_block_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let parsed = crate::ParsedSource {
            title: None,
            author: None,
            source_version_id,
            parser_version,
            blocks,
            page_count,
            needs_ocr,
            toc: if schema >= 34 {
                db.query_row(
                    "SELECT toc_json FROM source_parses WHERE id=?1",
                    [&id],
                    |r| r.get::<_, Option<String>>(0),
                )?
                .map(|json| serde_json::from_str(&json))
                .transpose()?
                .unwrap_or_default()
            } else {
                Vec::new()
            },
            cover: None,
            images: Vec::new(),
        };
        crate::sources::validate_parse(&parsed)?;
        crate::sources::validate_parse_format(
            &parsed,
            crate::sources::read_version(db, &parsed.source_version_id)?.format,
        )?;
    }
    let annotations = db
        .prepare("SELECT id FROM annotations")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in annotations {
        check("verify_annotations")?;
        let annotation = crate::source_annotations::read_annotation(db, &id)?;
        if annotation.source_version_id != annotation.locator.source_version_id()
            || annotation.comment.len() > 64 * 1024
            || annotation.locator.quote().is_empty()
        {
            return Err(Error::IntegrityFailure);
        }
        crate::source_annotations::validate_historical_locator(db, &annotation.locator)?;
    }
    let readings = db
        .prepare("SELECT source_id FROM reading_states")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in readings {
        let reading = crate::source_annotations::read_reading(db, &id)?;
        if let Some(locator) = &reading.locator {
            if locator.source_version_id() != reading.source_version_id {
                return Err(Error::IntegrityFailure);
            }
            crate::source_annotations::validate_historical_locator(db, locator)?;
        }
        if let Some(view) = &reading.pdf_view {
            crate::source_annotations::validate_pdf_reading_view(
                db,
                &crate::sources::read_version(db, &reading.source_version_id)?,
                reading.locator.as_ref(),
                view,
            )?;
        }
    }
    let ids = db
        .prepare("SELECT id FROM source_versions")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let version = crate::sources::read_version(db, &id)?;
        crate::source_jobs::validate_original_name(&version.original_name)?;
        if let Some(capture) = version.web_capture {
            if version.format != crate::SourceFormat::Html
                || version.origin_url.is_none()
                || capture.captured_at != version.captured_at
            {
                return Err(Error::IntegrityFailure);
            }
            chrono::DateTime::parse_from_rfc3339(&capture.captured_at)
                .map_err(|_| Error::IntegrityFailure)?;
            crate::sources::validate_web_capture(&crate::WebCaptureInput {
                final_url: capture.final_url,
                http_status: capture.http_status,
                content_type: capture.content_type,
            })?;
        }
        crate::document::valid_uuid(&version.source_id)?;
        if version.size == 0
            || version.size > crate::sources::MAX_SOURCE_BYTES
            || version.original_name.chars().count() > 300
        {
            return Err(Error::IntegrityFailure);
        }
    }
    let jobs = db
        .prepare("SELECT id FROM source_jobs")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in jobs {
        let job = crate::source_jobs::read_job(db, &id)?;
        crate::document::valid_uuid(&job.input.input_token)?;
    }
    Ok(())
}

pub(crate) fn validate_database(
    db: &Connection,
    check: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    check("database_integrity")?;
    // A matching migration_history alone cannot authorize attacker-added SQL.
    // Compare DDL to a fresh database before any writable restore connection.
    let mut expected = Connection::open_in_memory()?;
    let schema: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    crate::migrations::initialize_at(&mut expected, schema)?;
    let ddl = |db: &Connection| -> Result<Vec<(String, String, Option<String>)>> {
        Ok(db.prepare("SELECT type,name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")?
            .query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get::<_,Option<String>>(2)?.map(|sql|sql.replace("\r\n","\n")))))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    };
    if ddl(db)? != ddl(&expected)? {
        return Err(Error::UnsupportedDatabase);
    }
    if db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? != "ok" {
        return Err(Error::IntegrityFailure);
    }
    require_empty(db, "PRAGMA foreign_key_check")?;
    inspect_database(db)?;
    for sql in [
        "SELECT 1 FROM note_revisions WHERE length(CAST(document_json AS BLOB))>4194304 OR length(title)>300",
        "SELECT 1 FROM notes n LEFT JOIN note_revisions r ON r.revision_id=n.current_revision_id WHERE r.note_id IS NOT n.id OR r.revision IS NOT n.current_revision OR r.title IS NOT n.title OR n.current_revision != (SELECT max(revision) FROM note_revisions WHERE note_id=n.id)",
        "SELECT 1 FROM note_revisions r LEFT JOIN note_revisions p ON p.revision_id=r.parent_revision_id WHERE (r.revision=1 AND r.parent_revision_id IS NOT NULL) OR (r.revision>1 AND (p.note_id IS NOT r.note_id OR p.revision IS NOT r.revision-1))",
        "SELECT 1 FROM notes n LEFT JOIN objects o ON o.id=n.id WHERE o.kind IS NOT 'note' OR o.trashed IS NOT n.trashed",
        "SELECT 1 FROM collections c LEFT JOIN objects o ON o.id=c.id WHERE o.kind IS NOT 'collection'",
        "SELECT 1 FROM objects o WHERE (o.kind='note' AND NOT EXISTS(SELECT 1 FROM notes n WHERE n.id=o.id)) OR (o.kind='collection' AND NOT EXISTS(SELECT 1 FROM collections c WHERE c.id=o.id)) OR o.row_version<1",
        "SELECT 1 FROM memberships m JOIN objects o ON o.id=m.member_id WHERE o.kind='collection'",
        "SELECT 1 FROM note_revision_blocks r JOIN blocks b ON b.id=r.block_id JOIN note_revisions v ON v.revision_id=r.revision_id WHERE b.note_id!=v.note_id",
        "SELECT 1 FROM blocks b JOIN notes n ON n.id=b.note_id WHERE b.active != EXISTS(SELECT 1 FROM note_revision_blocks r WHERE r.revision_id=n.current_revision_id AND r.block_id=b.id)",
        "SELECT 1 FROM body_links b JOIN notes n ON n.id=b.source_id WHERE b.source_revision_id!=n.current_revision_id",
        "SELECT 1 FROM workspace_meta WHERE next_change_seq <= coalesce((SELECT max(change_seq) FROM command_receipts),0)",
        "SELECT 1 FROM command_receipts c JOIN note_revisions r ON r.revision_id=c.revision_id WHERE c.object_id!=r.note_id",
    ] {
        require_empty(db, sql)?;
    }
    if schema < 9 {
        require_empty(
            db,
            "SELECT 1 FROM objects WHERE kind NOT IN ('note','collection')",
        )?;
    } else {
        validate_sources(db, check)?;
    }
    if schema >= 16 {
        crate::generation_channels::validate_backup(db)?;
    }
    if schema >= 17 {
        require_empty(
            db,
            "SELECT 1 FROM object_tombstones t LEFT JOIN command_receipts c ON c.command_id=t.command_id WHERE c.command_kind!='trash.delete' OR EXISTS(SELECT 1 FROM objects o WHERE o.id=t.id) OR NOT EXISTS(SELECT 1 FROM json_each(c.result_json,'$.deleted_objects') j WHERE json_extract(j.value,'$.object_id')=t.id AND json_extract(j.value,'$.kind')=t.kind)",
        )?;
    }
    if schema >= 14 {
        crate::chat::validate_backup(db)?;
    }
    if schema >= 43 {
        crate::knowledge_tasks::validate_backup(db)?;
    }
    if schema >= 44 {
        crate::tag_sync::validate_backup(db)?;
    }
    if schema >= 13 {
        crate::review::validate_backup(db)?;
    }
    if schema >= 38 {
        crate::workspace_metadata::validate_backup(db)?;
    }
    if schema >= 25 {
        crate::canvases::validate_backup(db)?;
    }
    if schema >= 12 {
        crate::assistant::validate_backup(db)?;
        crate::index::validate_backup(db)?;
        crate::index_jobs::validate_backup(db)?;
    }
    let mut revisions =
        db.prepare("SELECT note_id,revision_id FROM note_revisions ORDER BY note_id,revision")?;
    let mut rows = revisions.query([])?;
    while let Some(row) = rows.next()? {
        check("verify_revisions")?;
        let note_id: String = row.get(0)?;
        let revision_id: String = row.get(1)?;
        crate::document::valid_uuid(&note_id)?;
        crate::document::valid_uuid(&revision_id)?;
        let note = notes::revision(db, &note_id, &revision_id)?;
        let nodes = note.document.nodes();
        let expected_blocks: BTreeSet<_> = nodes
            .iter()
            .filter(|n| !n.attrs.id.is_empty())
            .map(|n| n.attrs.id.clone())
            .collect();
        let actual_blocks = db
            .prepare("SELECT block_id FROM note_revision_blocks WHERE revision_id=?1")?
            .query_map([&revision_id], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<BTreeSet<_>, _>>()?;
        if expected_blocks != actual_blocks {
            return Err(Error::IntegrityFailure);
        }
        let expected_blobs: BTreeSet<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeType::Image)
            .map(|n| {
                (
                    n.attrs.id.clone(),
                    n.attrs.blob_hash.clone().unwrap_or_default(),
                    n.attrs.original_name.clone(),
                )
            })
            .collect();
        let actual_blobs = db.prepare("SELECT node_id,blob_hash,original_name FROM note_revision_blobs WHERE revision_id=?1")?.query_map([&revision_id], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, Option<String>>(2)?)))?.collect::<std::result::Result<BTreeSet<_>, _>>()?;
        if expected_blobs != actual_blobs {
            return Err(Error::IntegrityFailure);
        }
        let current: bool = db.query_row(
            "SELECT current_revision_id=?2 FROM notes WHERE id=?1",
            params![note_id, revision_id],
            |r| r.get(0),
        )?;
        if current {
            let expected: BTreeSet<_> = nodes
                .iter()
                .filter(|n| n.is_body_reference())
                .map(|n| {
                    (
                        n.attrs.occurrence_id.clone().unwrap_or_default(),
                        n.attrs.target_id.clone().unwrap_or_default(),
                        n.attrs.target_block_id.clone(),
                        n.plain_text(),
                        n.attrs.target_revision_id.clone(),
                    )
                })
                .collect();
            let sql = if schema >= 12 {
                "SELECT occurrence_id,target_id,target_block_id,context,target_revision_id FROM body_links WHERE source_id=?1"
            } else {
                "SELECT occurrence_id,target_id,target_block_id,context,NULL FROM body_links WHERE source_id=?1"
            };
            let actual = db
                .prepare(sql)?
                .query_map([&note_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                })?
                .collect::<std::result::Result<BTreeSet<_>, _>>()?;
            if expected != actual {
                return Err(Error::IntegrityFailure);
            }
        }
    }
    let mut views = db.prepare("SELECT filter_ast FROM saved_views")?;
    for ast in views.query_map([], |r| r.get::<_, String>(0))? {
        crate::queries::normalize_filter(serde_json::from_str(&ast?)?)?;
    }
    let mut tags = db.prepare("SELECT id,display_name,normalized_name FROM tags")?;
    for row in tags.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (id, display, normalized) = row?;
        crate::document::valid_uuid(&id)?;
        if crate::organization::normalize_name(&display)? != (display, normalized) {
            return Err(Error::IntegrityFailure);
        }
    }
    if schema >= 42 {
        let mut definitions =
            db.prepare("SELECT description,row_version,created_at,updated_at FROM tags")?;
        for definition in definitions.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })? {
            let (description, version, created, updated) = definition?;
            if description.chars().count() > 4000
                || description.contains('\0')
                || version == 0
                || [created, updated]
                    .into_iter()
                    .flatten()
                    .any(|value| chrono::DateTime::parse_from_rfc3339(&value).is_err())
            {
                return Err(Error::IntegrityFailure);
            }
        }
    }
    if schema >= 8 {
        let mut jobs = db.prepare("SELECT id FROM portable_jobs")?;
        for id in jobs.query_map([], |r| r.get::<_, String>(0))? {
            let job = crate::portable_jobs::read_job(db, &id?)?;
            if (job.status == crate::JobStatus::Succeeded) != job.result.is_some() {
                return Err(Error::IntegrityFailure);
            }
            match &job.input {
                crate::PortableJobInput::ExportNote {
                    note_id,
                    revision_id,
                    ..
                } => {
                    crate::document::valid_uuid(note_id)?;
                    crate::document::valid_uuid(revision_id)?;
                    crate::notes::revision(db, note_id, revision_id)?;
                }
                crate::PortableJobInput::ExportSource {
                    source_id,
                    version_id,
                } => {
                    crate::document::valid_uuid(source_id)?;
                    crate::document::valid_uuid(version_id)?;
                    if crate::sources::read_version(db, version_id)?.source_id != *source_id {
                        return Err(Error::IntegrityFailure);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

use pkms_core::*;
use serde_json::{Value, json};
use std::{io::Cursor, path::Path, time::Instant};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn index(db: &mut Library, object_id: &str, chunk_chars: usize) -> String {
    let profile = db
        .register_embedding_profile(
            &id(),
            EmbeddingProfileSpec {
                endpoint: None,
                model: "backup-cache-test".into(),
                revision: "1".into(),
                dimensions: 3,
                normalized: true,
                query_prefix: "".into(),
                passage_prefix: "".into(),
                tokenizer: "controlled-test".into(),
                chunker: "controlled-test".into(),
                max_tokens: 512,
                overlap_tokens: 0,
                provider: None,
                output_dimensions: None,
            },
        )
        .unwrap()
        .profile;
    let input = db.index_input(object_id).unwrap();
    let mut chunks = vec![];
    for block in &input.blocks {
        let chars: Vec<_> = block.text.chars().collect();
        for (position, text) in chars.chunks(chunk_chars).enumerate() {
            chunks.push(IndexChunkInput {
                chunk_id: format!("chunk_{:06}", chunks.len()),
                block_id: block.target.block_id.clone(),
                start_codepoint: (position * chunk_chars) as u32,
                end_codepoint: (position * chunk_chars + text.len()) as u32,
                text: text.iter().collect(),
                token_count: text.len() as u32,
            });
        }
    }
    let lease = db
        .begin_index_generation(
            &id(),
            IndexPlan {
                object_id: input.object_id,
                content_revision: input.content_revision,
                metadata_revision: input.metadata_revision,
                parser_revision: input.parser_revision,
                profile_id: profile.id,
                chunks,
            },
        )
        .unwrap();
    db.publish_index_generation(IndexPublish {
        generation_id: lease.generation.id.clone(),
        lease_token: lease.lease_token,
        profile_id: lease.generation.profile_id,
        chunk_count: lease.generation.chunk_count,
    })
    .unwrap();
    lease.generation.id
}

fn source(db: &mut Library) -> String {
    let accepted = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "cache.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let text = "a😀b中".as_bytes();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(text),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &accepted.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text).unwrap(),
    )
    .unwrap();
    version.source_id
}

fn reject_changed_second_chunk(db: &Library, root: &Path, generation: &str) {
    // Both chunks target the same block. The first warms the validation cache;
    // each forged second target must still fail even though its text is correct.
    let raw = rusqlite::Connection::open(root.join("library.sqlite3")).unwrap();
    let original: String = raw
        .query_row(
            "SELECT target_json FROM index_chunks WHERE generation_id=?1 AND id='chunk_000001'",
            [generation],
            |r| r.get(0),
        )
        .unwrap();
    for (field, value) in [
        ("parse_id", json!(id())),
        ("page_index", json!(99)),
        ("object_id", json!(id())),
        ("content_revision", json!(id())),
        ("metadata_revision", json!(999)),
        ("end_codepoint", json!(u32::MAX)),
    ] {
        let mut target: Value = serde_json::from_str(&original).unwrap();
        target[field] = value;
        raw.execute(
            "UPDATE index_chunks SET target_json=?2 WHERE generation_id=?1 AND id='chunk_000001'",
            rusqlite::params![generation, target.to_string()],
        )
        .unwrap();
        assert!(
            matches!(
                db.prepare_backup(root.join("invalid.zip"), &mut |_| Ok(())),
                Err(Error::IntegrityFailure)
            ),
            "forged {field} passed cached validation"
        );
    }
    raw.execute(
        "UPDATE index_chunks SET target_json=?2 WHERE generation_id=?1 AND id='chunk_000001'",
        rusqlite::params![generation, original],
    )
    .unwrap();
    db.prepare_backup(root.join("restored-valid.zip"), &mut |_| Ok(()))
        .unwrap();
}

#[test]
fn note_backup_cache_preserves_unicode_ranges_and_rejects_forged_later_identity() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("cache", "a😀b中").unwrap();
    let generation = index(&mut db, &note.id, 2);
    db.prepare_backup(root.path().join("valid.zip"), &mut |_| Ok(()))
        .unwrap();
    reject_changed_second_chunk(&db, root.path(), &generation);
}

#[test]
fn source_backup_cache_rechecks_parse_and_page_for_every_chunk() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let source = source(&mut db);
    let generation = index(&mut db, &source, 2);
    db.prepare_backup(root.path().join("valid.zip"), &mut |_| Ok(()))
        .unwrap();
    reject_changed_second_chunk(&db, root.path(), &generation);
}

#[test]
#[ignore = "manual 100k-chunk backup timing probe; all data stays in a temporary library"]
fn large_note_100k_chunk_backup_probe() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let body = "word ".repeat(760_000);
    let note = db.create_note("large backup probe", &body).unwrap();
    let generation = index(&mut db, &note.id, 38);
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    let count: u32 = raw
        .query_row(
            "SELECT count(*) FROM index_chunks WHERE generation_id=?1",
            [&generation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 100_000);
    let started = Instant::now();
    db.prepare_backup(root.path().join("large.zip"), &mut |_| Ok(()))
        .unwrap();
    eprintln!(
        "100000 chunks / {} original bytes: complete backup preparation {:?}",
        body.len(),
        started.elapsed()
    );
}

#[test]
fn remote_embedding_profile_survives_full_package_without_machine_credentials() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let note = db
        .create_note("远程向量备份", "原文😀和历史不依赖向量服务")
        .unwrap();
    let spec = EmbeddingProfileSpec {
        endpoint: Some("https://example.com/accounts/synthetic/ai/v1".into()),
        model: "@cf/baai/bge-m3".into(),
        revision: "cloudflare-hosted-bge-m3-v1".into(),
        dimensions: 1024,
        normalized: true,
        query_prefix: String::new(),
        passage_prefix: String::new(),
        tokenizer: "utf8-byte-upper-bound-v1".into(),
        chunker: "pkms-utf8-boundary-v1:title-newline:no-overlap".into(),
        max_tokens: 2048,
        overlap_tokens: 0,
        provider: None,
        output_dimensions: None,
    };
    let profile = db.register_embedding_profile(&id(), spec).unwrap().profile;
    let package = root.path().join("remote.zip");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    verify_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(destination).unwrap();
    let restored_profile = restored.index_status().unwrap().profile.unwrap();
    assert_eq!(
        serde_json::to_value(&restored_profile).unwrap(),
        serde_json::to_value(&profile).unwrap()
    );
    assert_eq!(restored.note(&note.id).unwrap().document, note.document);
    assert!(restored.active_index_generations().unwrap().is_empty());
    // A new machine must reconfigure credentials, which are never part of a core package.
    let mut archive = zip::ZipArchive::new(std::fs::File::open(package).unwrap()).unwrap();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).unwrap();
        assert!(!entry.name().contains("credential") && !entry.name().contains("vectors"));
    }
}

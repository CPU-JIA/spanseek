use pkms_core::*;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

#[test]
fn same_names_project_distinct_originals_and_bounded_current_parse_previews() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let mut results = Vec::new();
    for index in 0..2 {
        let url = format!("https://example.test/{index}");
        let input = ImportSource {
            input_token: id(),
            original_name: "同名资料.txt".into(),
            format: SourceFormat::Text,
            origin_url: Some(url.clone()),
            collection_id: None,
            add_version_to: None,
            base_row_version: None,
        };
        let job = db.accept_source_import(&id(), input).unwrap();
        let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
        let text = format!("{}{}\n末段", index, "中文摘要".repeat(80));
        let version = db
            .capture_source_original(
                &job.job_id,
                &lease.lease_token,
                &mut Cursor::new(text.as_bytes()),
                &mut |_| Ok(()),
            )
            .unwrap();
        let before = db.source(&version.source_id).unwrap();
        assert!(before.preview_text.is_none());
        db.publish_source_parse(
            &job.job_id,
            &lease.lease_token,
            parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
        )
        .unwrap();
        let summary = db.source(&version.source_id).unwrap();
        assert_eq!(summary.original_name, "同名资料.txt");
        assert_eq!(summary.origin_url, Some(url));
        assert_eq!(summary.content_hash, version.blob_hash);
        assert_eq!(
            summary.preview_text.unwrap(),
            text.chars().take(240).collect::<String>()
        );
        results.push(version.blob_hash);
    }
    assert_ne!(results[0], results[1]);
    let page = db
        .sources_page(SourcesPageRequest {
            filter: None,
            limit: 10,
            cursor: None,
            trashed: false,
            query: None,
        })
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(
        page.items
            .iter()
            .all(|s| s.preview_text.as_ref().unwrap().chars().count() == 240)
    );
}

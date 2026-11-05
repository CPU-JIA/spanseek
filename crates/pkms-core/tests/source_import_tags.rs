use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn input(title: &str) -> ImportSource {
    ImportSource {
        input_token: id(),
        original_name: title.into(),
        format: SourceFormat::Text,
        origin_url: None,
        collection_id: None,
        add_version_to: None,
        base_row_version: None,
    }
}

fn capture(db: &mut Library, text: &[u8], tag_names: Vec<String>) -> (SourceVersion, SourceJob) {
    let job = db
        .accept_source_import_with_tags(&id(), input("CAN 总线故障复盘.txt"), tag_names)
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(text),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text).unwrap(),
    )
    .unwrap();
    (version, db.source_job(&job.job_id).unwrap())
}

#[test]
fn import_job_round_trips_tags_and_retry_preserves_them() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let command = id();
    let request = input("SPI 时序与 CAN 总线故障排查.txt");
    let tags = vec!["故障排查".to_string(), "CAN 总线".to_string()];
    let accepted = db
        .accept_source_import_with_tags(&command, request.clone(), tags.clone())
        .unwrap();
    let job = db.source_job(&accepted.job_id).unwrap();
    assert_eq!(job.tag_names, tags);
    assert!(
        db.replay_source_import_with_tags(&command, &request, &tags)
            .unwrap()
            .is_some()
    );

    let canceled = db.cancel_source_job(&accepted.job_id).unwrap();
    assert_eq!(canceled.status, JobStatus::Canceled);
    let retried = db.retry_source_job(&id(), &accepted.job_id).unwrap();
    let retry = db.source_job(&retried.job_id).unwrap();
    assert_eq!(retry.tag_names, tags);
}

#[test]
fn duplicate_import_applies_new_tags_to_the_existing_source_and_finishes_it() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let text = "120Ω 终端电阻用于抑制 CAN 总线反射；错误帧计数升高时先检查采样点与总线负载。";
    let (first, _) = capture(&mut db, text.as_bytes(), Vec::new());
    assert_eq!(
        db.object_metadata(&first.source_id).unwrap().inbox_state,
        Some(InboxState::Pending)
    );

    let duplicate = db
        .accept_source_import_with_tags(
            &id(),
            input("CAN 总线故障复盘.txt"),
            vec!["故障排查".into(), "CAN 总线".into()],
        )
        .unwrap();
    let lease = db.claim_source_job(&duplicate.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &duplicate.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(version.source_id, first.source_id);
    let metadata = db.object_metadata(&first.source_id).unwrap();
    assert_eq!(metadata.inbox_state, Some(InboxState::Processed));
    let names: Vec<_> = metadata
        .tags
        .iter()
        .map(|tag| tag.display_name.as_str())
        .collect();
    assert_eq!(names, vec!["CAN 总线", "故障排查"]);
    let job = db.source_job(&duplicate.job_id).unwrap();
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(job.stage, "existing_source");
}

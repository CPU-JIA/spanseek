use pkms_core::*;
use rusqlite::Connection;
use std::{path::PathBuf, sync::OnceLock};
use tempfile::{Builder, TempDir};
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn temp() -> TempDir {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    let base = BASE.get_or_init(|| {
        let runs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local-archive/test-runs");
        std::fs::create_dir_all(&runs).unwrap();
        let root = std::env::var_os("PKMS_TEST_RUN_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                runs.join(format!(
                    "tag-sync-{}-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S-%f"),
                    std::process::id()
                ))
            });
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        assert!(root.starts_with(runs.canonicalize().unwrap()));
        root
    });
    Builder::new().prefix("core-").tempdir_in(base).unwrap()
}
fn tag(id: &str, name: &str) -> TagSyncPayload {
    TagSyncPayload::Tag {
        tag: SyncedTag {
            id: id.into(),
            display_name: name.into(),
            description: "说明".into(),
            created_at: None,
            updated_at: None,
            deleted_at: None,
        },
    }
}
fn assignment(object: &str, kind: ObjectKind, tags: &[String]) -> TagSyncPayload {
    let mut tags = tags.to_vec();
    tags.sort();
    tags.dedup();
    TagSyncPayload::TagAssignments {
        assignments: SyncedTagAssignments {
            object_id: object.into(),
            object_kind: kind,
            tag_ids: tags,
        },
    }
}
fn receive(db: &mut Library, payload: TagSyncPayload, version: i64) -> TagSyncApplyResult {
    db.tag_sync_receive(&payload.sync_object_id().unwrap(), payload, version)
        .unwrap()
}
fn create_tag(db: &mut Library, name: &str) -> TagDetails {
    db.tag_create(
        &id(),
        CreateTag {
            name: name.into(),
            description: "说明".into(),
        },
    )
    .unwrap()
    .tag
}
fn assign(db: &mut Library, object: &str, tag: &str) {
    let base = db.object_metadata(object).unwrap().row_version;
    db.assign_tag(
        &id(),
        AssignTag {
            targets: vec![MetadataTarget {
                object_id: object.into(),
                base_row_version: base,
            }],
            tag: TagSelector::Id { id: tag.into() },
        },
    )
    .unwrap();
}
fn remove(db: &mut Library, object: &str, tag: &str) {
    let base = db.object_metadata(object).unwrap().row_version;
    db.remove_tag(
        &id(),
        RemoveTag {
            targets: vec![MetadataTarget {
                object_id: object.into(),
                base_row_version: base,
            }],
            tag_id: tag.into(),
        },
    )
    .unwrap();
}
fn rename(db: &mut Library, tag_id: &str, name: &str) {
    let old = db.tag_details(tag_id).unwrap();
    db.tag_update(
        &id(),
        UpdateTag {
            id: tag_id.into(),
            base_row_version: old.row_version,
            name: name.into(),
            description: old.description,
        },
    )
    .unwrap();
}
fn delete(db: &mut Library, tag_id: &str) {
    let old = db.tag_details(tag_id).unwrap();
    db.tag_delete(
        &id(),
        DeleteTag {
            id: tag_id.into(),
            base_row_version: old.row_version,
            impact_token: old.impact_token,
        },
    )
    .unwrap();
}
fn payload(db: &Library, kind: TagSyncKind, id: &str) -> TagSyncPayload {
    db.tag_sync_snapshot(kind, id).unwrap().payload.unwrap()
}
fn note_at(db: &mut Library, note_id: &str) -> Note {
    db.create_document_with_id(
        &id(),
        note_id,
        CreateNote {
            title: "原始笔记".into(),
            document_schema: DOCUMENT_SCHEMA,
            document: NoteDocument::from_text(id(), "原始正文"),
            collection_id: None,
            base_collection_row_version: None,
        },
    )
    .unwrap()
    .note
}
fn tombstone(mut payload: TagSyncPayload) -> TagSyncPayload {
    let TagSyncPayload::Tag { tag } = &mut payload else {
        panic!("tag expected")
    };
    tag.deleted_at = Some("2026-11-05T04:00:00Z".into());
    payload
}

#[test]
fn wire_id_and_fingerprint_are_stable_strict_and_shared_by_both_instances() {
    let root = temp();
    let a = Library::open(root.path().join("a")).unwrap();
    let b = Library::open(root.path().join("b")).unwrap();
    let object = "01234567-89ab-4cde-8fab-0123456789ab".to_owned();
    let wire = tag_sync_object_id(TagSyncKind::TagAssignments, &object).unwrap();
    assert_ne!(wire, object);
    assert_eq!(Uuid::parse_str(&wire).unwrap().get_version_num(), 8);
    assert_eq!(wire, "f00eba22-707d-889a-a745-4e8a59b88dc4");
    assert_eq!(
        tag_sync_object_id(TagSyncKind::Tag, &object).unwrap(),
        "e54c5170-a8c6-85dc-be25-e01746388b5b"
    );
    let p = assignment(&object, ObjectKind::Note, &[]);
    let fp = p.fingerprint().unwrap();
    drop((a, b));
    assert_eq!(p.sync_object_id().unwrap(), wire);
    assert_eq!(p.fingerprint().unwrap(), fp);
    for original in [
        object.to_uppercase(),
        format!("{{{object}}}"),
        format!("urn:uuid:{object}"),
        object.replace('-', ""),
    ] {
        assert_ne!(
            tag_sync_object_id(TagSyncKind::Tag, &original).unwrap(),
            original
        );
        assert_ne!(
            tag_sync_object_id(TagSyncKind::Tag, &original).unwrap(),
            tag_sync_object_id(TagSyncKind::TagAssignments, &original).unwrap()
        );
        assert_ne!(
            tag_sync_object_id(TagSyncKind::TagAssignments, &original).unwrap(),
            wire
        );
    }
    let mut json = serde_json::to_value(tag(&id(), "名称")).unwrap();
    json["tag"]["future_field"] = true.into();
    assert!(serde_json::from_value::<TagSyncPayload>(json).is_err());
}

#[test]
fn old_null_times_replay_and_older_versions_never_rewrite_new_local_content() {
    let root = temp();
    let path = root.path().join("library");
    let mut db = Library::open(&path).unwrap();
    let tag_id = id();
    let original = tag(&tag_id, "最初");
    let first = receive(&mut db, original.clone(), 1);
    assert_eq!(first.status, TagSyncApplyStatus::Applied);
    let details = db.tag_details(&tag_id).unwrap();
    assert_eq!(details.created_at, None);
    assert_eq!(details.updated_at, None);
    let same = receive(&mut db, original.clone(), 1);
    assert_eq!(same.status, TagSyncApplyStatus::Unchanged);
    assert_eq!(db.tag_details(&tag_id).unwrap(), details);
    rename(&mut db, &tag_id, "本机新名称");
    let edited = db.tag_details(&tag_id).unwrap();
    let replay = receive(&mut db, original.clone(), 1);
    assert_eq!(replay.status, TagSyncApplyStatus::Unchanged);
    assert!(replay.snapshot.dirty);
    assert_eq!(db.tag_details(&tag_id).unwrap(), edited);
    assert!(matches!(
        db.tag_sync_receive(
            &tag_sync_object_id(TagSyncKind::Tag, &tag_id).unwrap(),
            tag(&tag_id, "同版不同内容"),
            1
        ),
        Err(Error::IdempotencyMismatch)
    ));
    drop(db);
    let db = Library::open_existing(path).unwrap();
    assert_eq!(db.tag_details(&tag_id).unwrap(), edited);
}

#[test]
fn dictionary_and_complete_assignments_round_trip_without_revising_note_bodies() {
    let root = temp();
    let mut a = Library::open(root.path().join("a")).unwrap();
    let mut b = Library::open(root.path().join("b")).unwrap();
    let note_id = id();
    let note_a = note_at(&mut a, &note_id);
    let note_b = note_at(&mut b, &note_id);
    let created = create_tag(&mut a, "共享标签");
    let tag_id = created.tag.id;
    let definition = payload(&a, TagSyncKind::Tag, &tag_id);
    a.tag_sync_acknowledge(definition.clone(), 1).unwrap();
    receive(&mut b, definition, 1);
    assign(&mut a, &note_id, &tag_id);
    let tags = payload(&a, TagSyncKind::TagAssignments, &note_id);
    a.tag_sync_acknowledge(tags.clone(), 1).unwrap();
    assert_eq!(receive(&mut b, tags, 1).status, TagSyncApplyStatus::Applied);
    assert_eq!(b.object_metadata(&note_id).unwrap().tags[0].id, tag_id);
    let before = b.object_metadata(&note_id).unwrap().row_version;
    rename(&mut a, &tag_id, "共享改名");
    let renamed = payload(&a, TagSyncKind::Tag, &tag_id);
    a.tag_sync_acknowledge(renamed.clone(), 2).unwrap();
    receive(&mut b, renamed, 2);
    assert_eq!(b.object_metadata(&note_id).unwrap().row_version, before + 1);
    assert_eq!(
        b.object_metadata(&note_id).unwrap().tags[0].display_name,
        "共享改名"
    );
    remove(&mut a, &note_id, &tag_id);
    let empty = payload(&a, TagSyncKind::TagAssignments, &note_id);
    assert!(
        matches!(&empty,TagSyncPayload::TagAssignments { assignments } if assignments.tag_ids.is_empty())
    );
    a.tag_sync_acknowledge(empty.clone(), 2).unwrap();
    receive(&mut b, empty, 2);
    assert!(b.object_metadata(&note_id).unwrap().tags.is_empty());
    assert_eq!(a.note(&note_id).unwrap().revision_id, note_a.revision_id);
    assert_eq!(b.note(&note_id).unwrap().revision_id, note_b.revision_id);
    assert_eq!(b.note(&note_id).unwrap().document, note_b.document);
}

#[test]
fn missing_targets_for_each_kind_are_durable_and_never_publish_empty_placeholders() {
    let root = temp();
    let path = root.path().join("library");
    let mut db = Library::open(&path).unwrap();
    let tag_id = id();
    receive(&mut db, tag(&tag_id, "先到标签"), 1);
    let mut pending = Vec::new();
    for kind in [
        ObjectKind::Note,
        ObjectKind::Source,
        ObjectKind::Annotation,
        ObjectKind::Collection,
    ] {
        let target = id();
        let p = assignment(&target, kind, std::slice::from_ref(&tag_id));
        let result = receive(&mut db, p.clone(), 1);
        assert_eq!(
            result.snapshot.pending.as_ref().unwrap().reason,
            TagSyncPendingReason::MissingObject
        );
        assert_eq!(result.snapshot.payload, None);
        assert_eq!(result.snapshot.local_fingerprint, None);
        pending.push((target, p));
    }
    drop(db);
    let mut db = Library::open_existing(&path).unwrap();
    for (target, p) in &pending {
        assert_eq!(
            db.tag_sync_snapshot(TagSyncKind::TagAssignments, target)
                .unwrap()
                .pending
                .unwrap()
                .payload,
            *p
        );
    }
    let note = note_at(&mut db, &pending[0].0);
    let result = db
        .tag_sync_retry(TagSyncKind::TagAssignments, &note.id, 1)
        .unwrap();
    assert_eq!(result.status, TagSyncApplyStatus::Applied);
    assert_eq!(db.object_metadata(&note.id).unwrap().tags[0].id, tag_id);
    assert_eq!(db.note(&note.id).unwrap().revision_id, note.revision_id);
}

#[test]
fn dependency_arrival_does_not_overwrite_edit_made_after_receipt_and_resolution_checks_both_sides()
{
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("目标", "保留正文").unwrap();
    let incoming_tag = id();
    let p = assignment(
        &note.id,
        ObjectKind::Note,
        std::slice::from_ref(&incoming_tag),
    );
    let first = receive(&mut db, p.clone(), 1);
    assert_eq!(
        first.snapshot.pending.unwrap().reason,
        TagSyncPendingReason::MissingTags
    );
    let local = create_tag(&mut db, "本机新增");
    assign(&mut db, &note.id, &local.tag.id);
    receive(&mut db, tag(&incoming_tag, "远端标签"), 1);
    let conflict = db
        .tag_sync_retry(TagSyncKind::TagAssignments, &note.id, 1)
        .unwrap();
    assert_eq!(
        conflict.snapshot.pending.unwrap().reason,
        TagSyncPendingReason::LocalChanges
    );
    assert!(matches!(
        db.tag_sync_keep_remote(TagSyncKind::TagAssignments, &note.id, 1, None, None),
        Err(Error::QueryExpired)
    ));
    let expected = db
        .tag_sync_snapshot(TagSyncKind::TagAssignments, &note.id)
        .unwrap()
        .local_fingerprint
        .unwrap();
    assert!(matches!(
        db.tag_sync_keep_remote(
            TagSyncKind::TagAssignments,
            &note.id,
            2,
            Some(&expected),
            None
        ),
        Err(Error::QueryExpired)
    ));
    let result = db
        .tag_sync_keep_remote(
            TagSyncKind::TagAssignments,
            &note.id,
            1,
            Some(&expected),
            None,
        )
        .unwrap();
    assert_eq!(result.status, TagSyncApplyStatus::Applied);
    assert_eq!(
        db.object_metadata(&note.id).unwrap().tags[0].id,
        incoming_tag
    );
    assert_eq!(db.note(&note.id).unwrap().revision_id, note.revision_id);
}

#[test]
fn same_name_different_ids_stays_pending_until_the_actual_collision_is_removed() {
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let local = create_tag(&mut db, "重复名称");
    let remote = id();
    let result = receive(&mut db, tag(&remote, "重复名称"), 1);
    assert_eq!(
        result.snapshot.pending.unwrap().reason,
        TagSyncPendingReason::NameConflict
    );
    let still = db
        .tag_sync_keep_remote(TagSyncKind::Tag, &remote, 1, None, None)
        .unwrap();
    assert_eq!(still.status, TagSyncApplyStatus::Pending);
    assert_eq!(
        still.snapshot.pending.unwrap().reason,
        TagSyncPendingReason::NameConflict
    );
    rename(&mut db, &local.tag.id, "本机保留名称");
    let result = db.tag_sync_retry(TagSyncKind::Tag, &remote, 1).unwrap();
    assert_eq!(result.status, TagSyncApplyStatus::Applied);
    assert_eq!(
        db.tag_details(&local.tag.id).unwrap().tag.display_name,
        "本机保留名称"
    );
    assert_eq!(
        db.tag_details(&remote).unwrap().tag.display_name,
        "重复名称"
    );
}

#[test]
fn remote_dictionary_deletion_protects_new_local_assignments_and_explicit_resolution_is_versioned()
{
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("笔记", "原文").unwrap();
    let tag_id = id();
    let original = tag(&tag_id, "受保护标签");
    receive(&mut db, original.clone(), 1);
    assign(&mut db, &note.id, &tag_id);
    let deletion = tombstone(original);
    let conflict = receive(&mut db, deletion.clone(), 2);
    assert_eq!(
        conflict.snapshot.pending.as_ref().unwrap().reason,
        TagSyncPendingReason::AssignmentChanges
    );
    let expected = conflict.snapshot.local_fingerprint.unwrap();
    let impact = conflict.snapshot.local_impact_token.unwrap();
    rename(&mut db, &tag_id, "确认以后又改名");
    assert!(matches!(
        db.tag_sync_keep_remote(TagSyncKind::Tag, &tag_id, 2, Some(&expected), Some(&impact)),
        Err(Error::QueryExpired)
    ));
    let expected = db.tag_sync_snapshot(TagSyncKind::Tag, &tag_id).unwrap();
    let applied = db
        .tag_sync_keep_remote(
            TagSyncKind::Tag,
            &tag_id,
            2,
            expected.local_fingerprint.as_deref(),
            expected.local_impact_token.as_deref(),
        )
        .unwrap();
    assert_eq!(applied.status, TagSyncApplyStatus::Applied);
    assert_eq!(applied.snapshot.payload, Some(deletion));
    assert!(db.object_metadata(&note.id).unwrap().tags.is_empty());
    assert_eq!(db.note(&note.id).unwrap().revision_id, note.revision_id);
    let stale = receive(&mut db, tag(&tag_id, "旧定义"), 1);
    assert_eq!(stale.status, TagSyncApplyStatus::IgnoredOlder);
}

#[test]
fn even_confirmed_remote_deletion_cannot_silently_broaden_a_saved_view() {
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let tag_id = id();
    let original = tag(&tag_id, "视图筛选");
    receive(&mut db, original.clone(), 1);
    let view = db
        .view_save(
            &id(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: "明确筛选".into(),
                filter_schema: 1,
                filter: NotesFilter {
                    tag_ids: vec![tag_id.clone()],
                    ..Default::default()
                },
                sort: NoteSort::UpdatedDesc,
                layout: ViewLayout::List,
                config_schema: 1,
                config: ViewConfig::default(),
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .view;
    let pending = receive(&mut db, tombstone(original), 2);
    assert_eq!(
        pending.snapshot.pending.as_ref().unwrap().reason,
        TagSyncPendingReason::ViewDependency
    );
    let keep = db
        .tag_sync_keep_remote(
            TagSyncKind::Tag,
            &tag_id,
            2,
            pending.snapshot.local_fingerprint.as_deref(),
            pending.snapshot.local_impact_token.as_deref(),
        )
        .unwrap();
    assert_eq!(keep.status, TagSyncApplyStatus::Pending);
    assert_eq!(
        db.view_get(&view.id).unwrap().filter.tag_ids,
        vec![tag_id.clone()]
    );
    db.view_remove(
        &id(),
        RemoveView {
            view_id: view.id,
            base_row_version: view.row_version,
        },
    )
    .unwrap();
    assert_eq!(
        db.tag_sync_retry(TagSyncKind::Tag, &tag_id, 2)
            .unwrap()
            .status,
        TagSyncApplyStatus::Applied
    );
}

#[test]
fn assignments_cannot_resurrect_a_deleted_tag_and_none_is_not_a_force_bypass() {
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("笔记", "正文").unwrap();
    let tag_id = id();
    let live = tag(&tag_id, "已删除");
    receive(&mut db, live.clone(), 1);
    receive(&mut db, tombstone(live.clone()), 2);
    let assignment = receive(
        &mut db,
        assignment(&note.id, ObjectKind::Note, std::slice::from_ref(&tag_id)),
        1,
    );
    assert_eq!(
        assignment.snapshot.pending.unwrap().reason,
        TagSyncPendingReason::DeletedTags
    );
    assert!(db.object_metadata(&note.id).unwrap().tags.is_empty());
    let restore = receive(&mut db, live, 3);
    assert_eq!(
        restore.snapshot.pending.unwrap().reason,
        TagSyncPendingReason::DeletedTag
    );
    assert!(matches!(
        db.tag_sync_keep_remote(TagSyncKind::Tag, &tag_id, 3, None, None),
        Err(Error::QueryExpired)
    ));
}

#[test]
fn late_acknowledgements_preserve_newer_pending_and_never_regress_a_newer_baseline() {
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let local = create_tag(&mut db, "待确认原名");
    let tag_id = local.tag.id;
    let staged = payload(&db, TagSyncKind::Tag, &tag_id);
    rename(&mut db, &tag_id, "发送期间新改名");
    let mut remote = staged.clone();
    let TagSyncPayload::Tag { tag } = &mut remote else {
        unreachable!()
    };
    tag.display_name = "另端名称".into();
    receive(&mut db, remote.clone(), 2);
    let ack = db.tag_sync_acknowledge(staged.clone(), 1).unwrap();
    assert_eq!(ack.remote_version, 2);
    assert_eq!(ack.base_version, 1);
    assert!(ack.dirty);
    assert_eq!(ack.pending.as_ref().unwrap().payload, remote);
    let current = ack.local_fingerprint.unwrap();
    let applied = db
        .tag_sync_keep_remote(TagSyncKind::Tag, &tag_id, 2, Some(&current), None)
        .unwrap();
    assert_eq!(applied.snapshot.base_version, 2);
    let late = db.tag_sync_acknowledge(staged, 1).unwrap();
    assert_eq!(late.base_version, 2);
    assert_eq!(late.base_fingerprint, remote.fingerprint().ok());
    assert!(!late.dirty);
}

#[test]
fn an_unsourced_acknowledgement_cannot_discard_a_missing_objects_received_payload() {
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let object = id();
    let received = assignment(&object, ObjectKind::Source, &[id()]);
    let before = receive(&mut db, received.clone(), 1).snapshot;
    assert!(matches!(
        db.tag_sync_acknowledge(received, 1),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(
        db.tag_sync_snapshot(TagSyncKind::TagAssignments, &object)
            .unwrap(),
        before
    );
}

#[test]
fn wrong_envelopes_unsorted_sets_and_wrong_object_kinds_do_not_change_metadata() {
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("笔记", "正文").unwrap();
    let metadata = db.object_metadata(&note.id).unwrap();
    let new_tag = tag(&id(), "有效名称");
    assert!(db.tag_sync_receive(&id(), new_tag, 1).is_err());
    assert!(
        db.tag_sync_receive(&note.id, tag(&note.id, "身份碰撞"), 1)
            .is_err()
    );
    let mut ids = vec![id(), id()];
    ids.sort();
    ids.reverse();
    let bad = TagSyncPayload::TagAssignments {
        assignments: SyncedTagAssignments {
            object_id: note.id.clone(),
            object_kind: ObjectKind::Note,
            tag_ids: ids,
        },
    };
    assert!(
        db.tag_sync_receive(&bad.sync_object_id().unwrap(), bad, 1)
            .is_err()
    );
    let wrong = receive(&mut db, assignment(&note.id, ObjectKind::Source, &[]), 1);
    assert_eq!(
        wrong.snapshot.pending.unwrap().reason,
        TagSyncPendingReason::WrongObjectKind
    );
    assert_eq!(db.object_metadata(&note.id).unwrap(), metadata);
}

#[test]
fn a_late_sql_failure_rolls_back_dictionary_metadata_and_sync_confirmation_together() {
    let root = temp();
    let path = root.path().join("library");
    let mut db = Library::open(&path).unwrap();
    let tag_id = id();
    let original = tag(&tag_id, "旧名");
    receive(&mut db, original, 1);
    let note = db.create_note("笔记", "正文").unwrap();
    receive(
        &mut db,
        assignment(&note.id, ObjectKind::Note, std::slice::from_ref(&tag_id)),
        1,
    );
    let before = db.tag_details(&tag_id).unwrap();
    let metadata = db.object_metadata(&note.id).unwrap();
    let sync = db.tag_sync_snapshot(TagSyncKind::Tag, &tag_id).unwrap();
    let raw = Connection::open(path.join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER test_tag_sync_abort BEFORE UPDATE OF base_version ON tag_sync_state WHEN NEW.base_version=2 BEGIN SELECT RAISE(ABORT,'controlled late tag failure'); END;").unwrap();
    assert!(matches!(
        db.tag_sync_receive(
            &tag_sync_object_id(TagSyncKind::Tag, &tag_id).unwrap(),
            tag(&tag_id, "未提交新名"),
            2
        ),
        Err(Error::Database(_))
    ));
    assert_eq!(db.tag_details(&tag_id).unwrap(), before);
    assert_eq!(db.object_metadata(&note.id).unwrap(), metadata);
    assert_eq!(
        db.tag_sync_snapshot(TagSyncKind::Tag, &tag_id).unwrap(),
        sync
    );
    raw.execute_batch("DROP TRIGGER test_tag_sync_abort;")
        .unwrap();
}

#[test]
fn complete_backup_preserves_pending_payload_baselines_and_exact_tombstones() {
    let root = temp();
    let mut db = Library::open(root.path().join("a")).unwrap();
    let tag_id = id();
    let live = tag(&tag_id, "墓碑资料");
    receive(&mut db, live.clone(), 1);
    let dead = tombstone(live);
    receive(&mut db, dead.clone(), 2);
    let absent = id();
    let pending = receive(
        &mut db,
        assignment(&absent, ObjectKind::Source, std::slice::from_ref(&tag_id)),
        1,
    )
    .snapshot;
    let package = root.path().join("backup.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let restored_path = root.path().join("restored");
    prepare_restore(&package, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored_path).unwrap();
    assert_eq!(
        restored
            .tag_sync_snapshot(TagSyncKind::TagAssignments, &absent)
            .unwrap(),
        pending
    );
    assert_eq!(
        restored
            .tag_sync_snapshot(TagSyncKind::Tag, &tag_id)
            .unwrap()
            .payload,
        Some(dead)
    );
}

#[test]
fn local_deletion_keeps_outbound_empty_assignments_and_dictionary_tombstone() {
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("笔记", "正文").unwrap();
    let tag = create_tag(&mut db, "本机标签");
    assign(&mut db, &note.id, &tag.tag.id);
    let definition = payload(&db, TagSyncKind::Tag, &tag.tag.id);
    let assigned = payload(&db, TagSyncKind::TagAssignments, &note.id);
    db.tag_sync_acknowledge(definition, 1).unwrap();
    db.tag_sync_acknowledge(assigned, 1).unwrap();
    delete(&mut db, &tag.tag.id);
    let empty = db
        .tag_sync_snapshot(TagSyncKind::TagAssignments, &note.id)
        .unwrap();
    assert!(empty.dirty);
    assert!(
        matches!(empty.payload,Some(TagSyncPayload::TagAssignments { assignments }) if assignments.tag_ids.is_empty())
    );
    let dead = db.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id).unwrap();
    assert!(dead.dirty);
    assert!(matches!(dead.payload,Some(TagSyncPayload::Tag { tag }) if tag.deleted_at.is_some()));
    assert_eq!(db.note(&note.id).unwrap().revision_id, note.revision_id);
}

#[test]
fn bounded_pages_cover_candidates_and_reject_reusing_cursor_for_another_scope() {
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    for n in 0..13 {
        create_tag(&mut db, &format!("标签{n:02}"));
    }
    let first = db
        .tag_sync_page(TagSyncPageRequest {
            limit: 5,
            after: None,
            pending_only: false,
        })
        .unwrap();
    assert_eq!(first.items.len(), 5);
    let cursor = first.next_cursor.clone();
    assert!(
        db.tag_sync_page(TagSyncPageRequest {
            limit: 5,
            after: cursor.clone(),
            pending_only: true
        })
        .is_err()
    );
    let second = db
        .tag_sync_page(TagSyncPageRequest {
            limit: 5,
            after: cursor,
            pending_only: false,
        })
        .unwrap();
    let third = db
        .tag_sync_page(TagSyncPageRequest {
            limit: 5,
            after: second.next_cursor,
            pending_only: false,
        })
        .unwrap();
    assert_eq!(third.items.len(), 3);
    assert!(third.next_cursor.is_none());
    let mut ids = first
        .items
        .into_iter()
        .chain(second.items)
        .chain(third.items)
        .map(|r| r.entity_id)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 13);
}

#[test]
fn all_four_real_entity_kinds_materialize_without_copying_or_rewriting_source_data() {
    let root = temp();
    let mut a = Library::open(root.path().join("a")).unwrap();
    let note = a.create_note("正文笔记", "原文保留").unwrap();
    let collection = a.create_collection("资料分类").unwrap();
    let text = "需要定位的原文摘录。";
    let job = a
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "同步资料.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: Some(collection.id.clone()),
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = a.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = a
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    a.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
    )
    .unwrap();
    let block = a
        .source_text_page(&version.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    let annotation = a
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: version.id.clone(),
                    block_id: block.id,
                    start_codepoint: 0,
                    end_codepoint: block.text.chars().count() as u32,
                    quote: block.text,
                    prefix: String::new(),
                    suffix: String::new(),
                },
                comment: "真实摘录".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let package = root.path().join("objects.pkmsbackup");
    a.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("b");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut b = Library::open_existing(destination).unwrap();
    let created = create_tag(&mut a, "四类实体标签");
    receive(&mut b, payload(&a, TagSyncKind::Tag, &created.tag.id), 1);
    for (object, kind) in [
        (&note.id, ObjectKind::Note),
        (&collection.id, ObjectKind::Collection),
        (&version.source_id, ObjectKind::Source),
        (&annotation.id, ObjectKind::Annotation),
    ] {
        assign(&mut a, object, &created.tag.id);
        let applied = receive(&mut b, payload(&a, TagSyncKind::TagAssignments, object), 1);
        assert_eq!(applied.status, TagSyncApplyStatus::Applied);
        let metadata = b.object_metadata(object).unwrap();
        assert_eq!(metadata.kind, kind);
        assert_eq!(metadata.tags[0].id, created.tag.id);
    }
    assert_eq!(b.note(&note.id).unwrap().revision_id, note.revision_id);
    assert_eq!(b.note(&note.id).unwrap().document, note.document);
    assert_eq!(
        b.source_text_page(&version.id, None, 0, 1).unwrap().blocks[0].text,
        text
    );
    remove(&mut a, &version.source_id, &created.tag.id);
    receive(
        &mut b,
        payload(&a, TagSyncKind::TagAssignments, &version.source_id),
        2,
    );
    assert!(
        b.object_metadata(&version.source_id)
            .unwrap()
            .tags
            .is_empty()
    );
    assert_eq!(
        b.object_metadata(&annotation.id).unwrap().tags[0].id,
        created.tag.id
    );
}

#[test]
fn legacy_uuid_text_remains_the_exact_database_identity_across_sync_and_restore() {
    let root = temp();
    let mut a = Library::open(root.path().join("a")).unwrap();
    let mut b = Library::open(root.path().join("b")).unwrap();
    let original = "01234567-89ab-4cde-8fab-0123456789ab";
    let variants = [
        original.to_uppercase(),
        format!("{{{original}}}"),
        format!("urn:uuid:{original}"),
        original.replace('-', ""),
    ];
    let tag_id = "FEDCBA98-7654-4321-8ABC-FEDCBA987654";
    receive(&mut a, tag(tag_id, "保留原始身份"), 1);
    receive(&mut b, tag(tag_id, "保留原始身份"), 1);
    for original in &variants {
        note_at(&mut a, original);
        let before = note_at(&mut b, original);
        assign(&mut a, original, tag_id);
        let sent = payload(&a, TagSyncKind::TagAssignments, original);
        assert_eq!(sent.entity_id(), original);
        assert_eq!(receive(&mut b, sent, 1).status, TagSyncApplyStatus::Applied);
        assert_eq!(b.object_metadata(original).unwrap().tags[0].id, tag_id);
        assert_eq!(b.note(original).unwrap().revision_id, before.revision_id);
    }
    let package = root.path().join("legacy-identities.pkmsbackup");
    b.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(destination).unwrap();
    for original in &variants {
        assert_eq!(restored.note(original).unwrap().id, *original);
        assert_eq!(
            restored
                .tag_sync_snapshot(TagSyncKind::TagAssignments, original)
                .unwrap()
                .entity_id,
            *original
        );
    }
}

#[test]
fn a_future_field_in_deferred_knowledge_is_rejected_before_a_backup_is_published() {
    let root = temp();
    let path = root.path().join("library");
    let mut db = Library::open(&path).unwrap();
    let object = id();
    let pending = assignment(&object, ObjectKind::Source, &[id()]);
    receive(&mut db, pending, 1);
    let raw = Connection::open(path.join("library.sqlite3")).unwrap();
    raw.execute("UPDATE tag_sync_state SET pending_payload_json=json_set(pending_payload_json,'$.assignments.future_field',1)", []).unwrap();
    let package = root.path().join("must-not-publish.pkmsbackup");
    assert!(db.create_backup(&package, &mut |_| Ok(())).is_err());
    assert!(!package.exists());
}

#[test]
fn review_entity_ids_equal_to_other_wire_ids_remain_safe_for_live_and_pending_entities() {
    for missing in [false, true] {
        for tag_first in [false, true] {
            let root = temp();
            let mut db = Library::open(root.path()).unwrap();
            let object = id();
            if !missing {
                note_at(&mut db, &object);
            }
            let assignment = assignment(
                &object,
                if missing {
                    ObjectKind::Source
                } else {
                    ObjectKind::Note
                },
                &[],
            );
            let assignment_wire = assignment.sync_object_id().unwrap();
            let dictionary = tag(&assignment_wire, "原始ID等于另一种wire");
            if tag_first {
                receive(&mut db, dictionary.clone(), 1);
                receive(&mut db, assignment, 1);
            } else {
                receive(&mut db, assignment, 1);
                receive(&mut db, dictionary.clone(), 1);
            }
            let package = root.path().join("must-stay-backupable.pkmsbackup");
            db.create_backup(&package, &mut |_| Ok(())).unwrap();
            assert_ne!(dictionary.sync_object_id().unwrap(), assignment_wire);
            assert_eq!(
                db.tag_details(&assignment_wire).unwrap().tag.id,
                assignment_wire
            );
            receive(&mut db, tombstone(dictionary), 2);
            db.create_backup(root.path().join("with-tombstone.pkmsbackup"), &mut |_| {
                Ok(())
            })
            .unwrap();
        }
    }
    let reserved = "7d3f2c1a-6b5e-4f10-9a8b-0c1d2e3f4a5b";
    for kind in [TagSyncKind::Tag, TagSyncKind::TagAssignments] {
        let wire = tag_sync_object_id(kind, reserved).unwrap();
        assert_ne!(wire, reserved);
        assert_eq!(Uuid::parse_str(&wire).unwrap().get_version_num(), 8);
    }
}

#[test]
fn review_existing_note_view_and_canvas_wire_ids_are_rejected_before_acceptance() {
    for kind in [TagSyncKind::Tag, TagSyncKind::TagAssignments] {
        for blocker in ["note", "view", "canvas"] {
            let root = temp();
            let mut db = Library::open(root.path()).unwrap();
            let entity = id();
            let incoming = if kind == TagSyncKind::Tag {
                tag(&entity, "不能覆盖既有对象")
            } else {
                assignment(&entity, ObjectKind::Source, &[])
            };
            let wire = incoming.sync_object_id().unwrap();
            match blocker {
                "note" => {
                    note_at(&mut db, &wire);
                }
                "view" => {
                    let view = db
                        .view_save(
                            &id(),
                            SaveView {
                                view_id: None,
                                base_row_version: None,
                                name: "原视图".into(),
                                filter_schema: 1,
                                filter: NotesFilter::default(),
                                sort: NoteSort::UpdatedDesc,
                                layout: ViewLayout::List,
                                config_schema: 1,
                                config: ViewConfig::default(),
                                is_pinned: false,
                                pin_position: None,
                            },
                        )
                        .unwrap()
                        .view;
                    db.view_apply_remote(SavedView {
                        id: wire.clone(),
                        name: "保留的视图".into(),
                        ..view
                    })
                    .unwrap();
                }
                "canvas" => {
                    let canvas = db
                        .canvas_save(
                            &id(),
                            SaveCanvas {
                                canvas_id: None,
                                base_row_version: None,
                                name: "原画布".into(),
                                config_schema: 1,
                                nodes: Vec::new(),
                                is_pinned: false,
                                pin_position: None,
                            },
                        )
                        .unwrap()
                        .canvas;
                    db.canvas_apply_remote(Canvas {
                        id: wire.clone(),
                        name: "保留的画布".into(),
                        ..canvas
                    })
                    .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    db.tag_sync_receive(&wire, incoming, 1),
                    Err(Error::InvalidInput(_))
                ),
                "{kind:?}/{blocker}"
            );
            assert!(matches!(
                db.tag_sync_snapshot(kind, &entity),
                Err(Error::NotFound)
            ));
            db.create_backup(root.path().join("still-valid.pkmsbackup"), &mut |_| Ok(()))
                .unwrap();
        }
    }
}

#[test]
fn review_remote_delete_confirmation_cannot_remove_assignments_added_after_preview() {
    let root = temp();
    let mut db = Library::open(root.path()).unwrap();
    let first = db.create_note("先前分配", "第一份原文").unwrap();
    let second = db.create_note("预览后分配", "第二份原文").unwrap();
    let tag_id = id();
    let original = tag(&tag_id, "待删除标签");
    receive(&mut db, original.clone(), 1);
    assign(&mut db, &first.id, &tag_id);
    let preview = receive(&mut db, tombstone(original), 2).snapshot;
    assert_eq!(
        preview.pending.as_ref().unwrap().reason,
        TagSyncPendingReason::AssignmentChanges
    );
    assign(&mut db, &second.id, &tag_id);
    let current = db.tag_sync_snapshot(TagSyncKind::Tag, &tag_id).unwrap();
    assert_eq!(preview.local_fingerprint, current.local_fingerprint);
    assert_ne!(preview.local_impact_token, current.local_impact_token);
    assert!(matches!(
        db.tag_sync_keep_remote(
            TagSyncKind::Tag,
            &tag_id,
            2,
            preview.local_fingerprint.as_deref(),
            preview.local_impact_token.as_deref()
        ),
        Err(Error::QueryExpired)
    ));
    assert!(matches!(
        db.tag_sync_keep_remote(
            TagSyncKind::Tag,
            &tag_id,
            2,
            current.local_fingerprint.as_deref(),
            None
        ),
        Err(Error::QueryExpired)
    ));
    assert_eq!(db.object_metadata(&first.id).unwrap().tags[0].id, tag_id);
    assert_eq!(db.object_metadata(&second.id).unwrap().tags[0].id, tag_id);
    let accepted = db
        .tag_sync_keep_remote(
            TagSyncKind::Tag,
            &tag_id,
            2,
            current.local_fingerprint.as_deref(),
            current.local_impact_token.as_deref(),
        )
        .unwrap();
    assert_eq!(accepted.status, TagSyncApplyStatus::Applied);
    assert_eq!(accepted.changed_objects.len(), 2);
    assert!(db.object_metadata(&first.id).unwrap().tags.is_empty());
    assert!(db.object_metadata(&second.id).unwrap().tags.is_empty());
    assert_eq!(db.note(&first.id).unwrap().revision_id, first.revision_id);
    assert_eq!(db.note(&second.id).unwrap().revision_id, second.revision_id);
}

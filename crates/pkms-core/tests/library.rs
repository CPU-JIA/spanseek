use pkms_core::{Error, Library};
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn committed_revisions_survive_reopen_and_restore_keeps_identity() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let original = library.create_note("知识管理", "第一行\n第二行🙂").unwrap();
    let edited = library
        .save_note(&original.id, 1, "新的标题", "第二版")
        .unwrap();
    assert_eq!(
        edited.document.content[0].attrs,
        original.document.content[0].attrs
    );
    drop(library);

    let mut reopened = Library::open_existing(dir.path()).unwrap();
    assert_eq!(reopened.note(&original.id).unwrap(), edited);
    let restored = reopened.restore_revision(&original.id, 2, 1).unwrap();
    assert_eq!(restored.id, original.id);
    assert_eq!(restored.revision, 3);
    assert_eq!(restored.title, original.title);
    assert_eq!(restored.document, original.document);
    let history = reopened.revisions(&original.id).unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].document, original.document);
    assert_eq!(history[1].document, edited.document);
}

#[test]
fn two_connections_cannot_overwrite_a_newer_revision() {
    let dir = tempdir().unwrap();
    let mut first = Library::open(dir.path()).unwrap();
    let note = first.create_note("并发", "旧正文").unwrap();
    let mut second = Library::open_existing(dir.path()).unwrap();
    let stale = second.note(&note.id).unwrap();
    let saved = first.save_note(&note.id, 1, "成功", "苹果").unwrap();
    assert!(matches!(
        second.save_note(&note.id, stale.revision, "覆盖", "香蕉"),
        Err(Error::Conflict {
            expected: 1,
            actual: 2
        })
    ));
    assert_eq!(second.note(&note.id).unwrap(), saved);
    assert_eq!(second.revisions(&note.id).unwrap().len(), 2);
    assert!(second.search("香蕉", 10).unwrap().is_empty());
    assert_eq!(second.search("苹果", 10).unwrap().len(), 1);
    assert!(matches!(
        second.restore_revision(&note.id, 1, 1),
        Err(Error::Conflict { .. })
    ));
}

#[test]
fn simultaneous_saves_have_exactly_one_winner() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let note = library.create_note("并发", "起点").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = ["苹果", "香蕉"]
        .into_iter()
        .map(|body| {
            let path = dir.path().to_path_buf();
            let id = note.id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut connection = Library::open_existing(path).unwrap();
                barrier.wait();
                connection.save_note(&id, 1, "并发", body)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(Error::Conflict { .. })))
            .count(),
        1
    );
    assert_eq!(library.revisions(&note.id).unwrap().len(), 2);
}

#[test]
fn index_failure_rolls_back_body_and_history() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let note = library.create_note("原版", "保持原样").unwrap();
    // Deliberately make the final indexing step fail after the revision INSERT.
    let raw = Connection::open(dir.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("DROP TABLE note_fts;").unwrap();
    assert!(
        library
            .save_note(&note.id, 1, "不应保存", "新正文")
            .is_err()
    );
    assert_eq!(library.note(&note.id).unwrap(), note);
    assert_eq!(library.revisions(&note.id).unwrap().len(), 1);
}

#[test]
fn memberships_and_links_follow_identity_not_title_or_location() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let note = library.create_note("目标", "正文").unwrap();
    let source = library.create_note("引用者", "参考目标").unwrap();
    let a = library.create_collection("学习").unwrap();
    let b = library.create_collection("写作").unwrap();
    for collection in [&a, &b] {
        library.add_to_collection(&collection.id, &note.id).unwrap();
        library.add_to_collection(&collection.id, &note.id).unwrap();
        assert_eq!(library.collection_notes(&collection.id).unwrap().len(), 1);
    }
    library
        .link_notes(&source.id, &note.id, "用于论证")
        .unwrap();
    library.remove_from_collection(&a.id, &note.id).unwrap();
    library.save_note(&note.id, 1, "已改名", "正文").unwrap();
    assert!(library.collection_notes(&a.id).unwrap().is_empty());
    assert_eq!(library.collection_notes(&b.id).unwrap()[0].id, note.id);
    let links = library.backlinks(&note.id).unwrap();
    assert_eq!(links[0].source_id, source.id);
    assert_eq!(links[0].context, "用于论证");
    assert_eq!(library.note(&note.id).unwrap().title, "已改名");
    library.set_trashed(&source.id, true).unwrap();
    assert!(library.backlinks(&note.id).unwrap().is_empty());
    library.set_trashed(&source.id, false).unwrap();
    assert_eq!(library.backlinks(&note.id).unwrap(), links);
}

#[test]
fn chinese_search_tracks_edits_trash_and_restore() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let note = library
        .create_note("中文", "个人知识管理 Rust SQLite 苹果")
        .unwrap();
    for query in ["知识", "管理", "RUST", "知识 SQLite"] {
        assert_eq!(library.search(query, 10).unwrap()[0].note.id, note.id);
    }
    library.save_note(&note.id, 1, "中文", "香蕉 Rust").unwrap();
    assert!(library.search("苹果", 10).unwrap().is_empty());
    assert_eq!(library.search("香蕉", 10).unwrap().len(), 1);
    library.set_trashed(&note.id, true).unwrap();
    assert!(library.search("香蕉", 10).unwrap().is_empty());
    assert!(library.list_notes().unwrap().is_empty());
    assert!(matches!(
        library.save_note(&note.id, 2, "标题", "正文"),
        Err(Error::Trashed)
    ));
    library.set_trashed(&note.id, false).unwrap();
    assert_eq!(library.search("香蕉", 10).unwrap().len(), 1);
    assert!(library.search("", 10).unwrap().is_empty());
    assert!(library.search("\"*() : OR NOT", 10).is_ok());
    assert!(library.search("香蕉", 0).is_err());
}

#[test]
fn nonexistent_references_and_invalid_text_do_not_create_objects() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    assert!(library.create_note("  ", "正文").is_err());
    let note = library.create_note("有效", "").unwrap();
    let collection = library.create_collection("主题").unwrap();
    assert!(library.link_notes(&note.id, "absent", "上下文").is_err());
    assert!(library.link_notes("absent", &note.id, "上下文").is_err());
    assert!(library.add_to_collection(&collection.id, "absent").is_err());
    assert!(library.add_to_collection("absent", &note.id).is_err());
    assert!(library.collection_notes(&collection.id).unwrap().is_empty());
    assert!(library.backlinks(&note.id).unwrap().is_empty());
    assert_eq!(library.list_notes().unwrap().len(), 1);
}

#[test]
fn online_backup_is_independent_complete_and_never_overwrites() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let note = library.create_note("知识", "第一版").unwrap();
    let saved = library.save_note(&note.id, 1, "知识", "第二版").unwrap();
    let source = library.create_note("出处", "引用").unwrap();
    let trashed = library.create_note("回收", "保留").unwrap();
    library.set_trashed(&trashed.id, true).unwrap();
    let collection = library.create_collection("主题").unwrap();
    library.add_to_collection(&collection.id, &note.id).unwrap();
    library
        .link_notes(&source.id, &note.id, "参考上下文")
        .unwrap();
    let backup = dir.path().join("snapshot.sqlite3");
    library.backup_to(&backup).unwrap();
    let bytes = std::fs::read(&backup).unwrap();
    assert!(library.backup_to(&backup).is_err());
    assert_eq!(std::fs::read(&backup).unwrap(), bytes);
    library.save_note(&note.id, 2, "未来", "第三版").unwrap();
    // Only copy the published snapshot; no source database or WAL accompanies it.
    let restored_dir = tempdir().unwrap();
    std::fs::copy(&backup, restored_dir.path().join("library.sqlite3")).unwrap();
    let restored = Library::open_existing(restored_dir.path()).unwrap();
    assert_eq!(restored.note(&note.id).unwrap(), saved);
    assert_eq!(restored.revisions(&note.id).unwrap().len(), 2);
    assert_eq!(
        restored.collection_notes(&collection.id).unwrap()[0].id,
        note.id
    );
    assert_eq!(
        restored.backlinks(&note.id).unwrap()[0].context,
        "参考上下文"
    );
    assert_eq!(restored.search("知识", 10).unwrap()[0].note.id, note.id);
    assert!(restored.note(&trashed.id).unwrap().trashed);
    assert_eq!(restored.list_notes().unwrap().len(), 2);
}

#[test]
fn unrelated_and_future_databases_are_rejected_unchanged() {
    let dir = tempdir().unwrap();
    let unrelated = dir.path().join("unrelated.sqlite3");
    {
        let raw = Connection::open(&unrelated).unwrap();
        raw.execute_batch(
            "CREATE TABLE personal_data(value TEXT); INSERT INTO personal_data VALUES('keep');",
        )
        .unwrap();
    }
    let bytes = std::fs::read(&unrelated).unwrap();
    assert!(matches!(
        Library::open_database(&unrelated),
        Err(Error::UnsupportedDatabase)
    ));
    assert_eq!(std::fs::read(&unrelated).unwrap(), bytes);
    let future = dir.path().join("future.sqlite3");
    {
        let raw = Connection::open(&future).unwrap();
        raw.execute_batch("PRAGMA application_id=1347112275; PRAGMA user_version=99;")
            .unwrap();
    }
    let bytes = std::fs::read(&future).unwrap();
    assert!(matches!(
        Library::open_database(&future),
        Err(Error::UnsupportedDatabase)
    ));
    assert_eq!(std::fs::read(&future).unwrap(), bytes);
}

#[test]
fn open_existing_never_initializes_missing_or_empty_workspaces() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("typo");
    assert!(Library::open_existing(&missing).is_err());
    assert!(!missing.exists());
    assert!(Library::open_existing(dir.path()).is_err());
    assert!(!dir.path().join("library.sqlite3").exists());
    std::fs::write(dir.path().join("library.sqlite3"), b"").unwrap();
    assert!(Library::open_existing(dir.path()).is_err());
    assert_eq!(
        std::fs::metadata(dir.path().join("library.sqlite3"))
            .unwrap()
            .len(),
        0
    );
}

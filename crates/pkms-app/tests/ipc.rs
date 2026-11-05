use pkms_app::*;
use pkms_core::{
    CopyNote, CreateNote, DeleteTrash, NoteDocument, SaveNote, SetObjectFlag, TrashPreviewRequest,
    TrashSelection,
};
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn request(identity: Option<WorkspaceIdentity>, command: DesktopCommand) -> DesktopRequest {
    DesktopRequest {
        api_version: 1,
        request_id: id(),
        identity,
        command,
    }
}
fn workspace(app: &mut AppService, path: &Path, create: bool) -> WorkspaceIdentity {
    let purpose = if create {
        GrantPurpose::CreateWorkspace
    } else {
        GrantPurpose::OpenWorkspace
    };
    let kind = if create {
        OperationKind::CreateWorkspace
    } else {
        OperationKind::OpenWorkspace
    };
    let grant = app.grant_path(purpose, path).unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), kind, &grant.token)
        .unwrap();
    let result = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(result.status, OperationStatus::Succeeded);
    result.result.unwrap().identity
}
fn app_state(app: &mut AppService) -> DesktopAppState {
    let DesktopData::AppState(state) = app
        .dispatch(request(None, DesktopCommand::AppState))
        .unwrap()
        .data
    else {
        panic!()
    };
    state
}
fn recent_identities(app: &mut AppService) -> Vec<WorkspaceIdentity> {
    app_state(app)
        .workspaces
        .into_iter()
        .map(|entry| entry.identity)
        .collect()
}
fn create_note(app: &mut AppService, identity: &WorkspaceIdentity, title: &str) -> String {
    let DesktopData::NoteCommit(created) = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::NotesCreate {
                command_id: id(),
                note: CreateNote {
                    collection_id: None,
                    base_collection_row_version: None,
                    title: title.into(),
                    document_schema: 2,
                    document: NoteDocument::from_text(id(), "正文"),
                },
            },
        ))
        .unwrap()
        .data
    else {
        panic!()
    };
    created.note.id
}
fn recent_object_ids(app: &mut AppService, identity: &WorkspaceIdentity) -> Vec<String> {
    let DesktopData::RecentObjects(items) = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::RecentObjectsList,
        ))
        .unwrap()
        .data
    else {
        panic!()
    };
    items.into_iter().map(|item| item.object_id).collect()
}
fn picture() -> Vec<u8> {
    // Real 1x1 RGBA PNG fixture, including valid CRCs.
    vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 96, 96, 96, 248, 15,
        0, 1, 4, 1, 0, 95, 229, 195, 75, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
}

#[test]
fn chat_wire_replays_without_duplicate_messages_and_enforces_workspace_identity() {
    use pkms_core::{ChatPurpose, ChatRunStatus, CreateConversation, StartChatRun};
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("chat"), true);
    let create = request(
        Some(identity.clone()),
        DesktopCommand::ChatConversationsCreate {
            command_id: id(),
            request: CreateConversation {
                title: "自由对话".into(),
            },
        },
    );
    let created = app.dispatch(create.clone()).unwrap();
    let replayed = app.dispatch(create).unwrap();
    let mut expected = serde_json::to_value(&created.data).unwrap();
    expected["payload"]["receipt"]["replayed"] = json!(true);
    assert_eq!(expected, serde_json::to_value(&replayed.data).unwrap());
    let DesktopData::ChatConversationMutation(created) = created.data else {
        panic!()
    };
    let conversation_id = created.conversation.id;
    let start = request(
        Some(identity.clone()),
        DesktopCommand::ChatRunsStart {
            command_id: id(),
            request: StartChatRun {
                conversation_id: conversation_id.clone(),
                base_row_version: created.conversation.row_version,
                text: "请解释为什么资料也可能有错。".into(),
                purpose: ChatPurpose::Review,
                materials: vec![],
                scope: Default::default(),
                use_knowledge_tools: false,
                options: None,
            },
        },
    );
    let wire = serde_json::to_value(&start).unwrap();
    assert_eq!(wire["command"]["kind"], "chat_runs_start");
    let first = app.dispatch(DesktopRequest::parse(wire).unwrap()).unwrap();
    let second = app.dispatch(start).unwrap();
    let mut expected = serde_json::to_value(&first.data).unwrap();
    expected["payload"]["receipt"]["replayed"] = json!(true);
    assert_eq!(expected, serde_json::to_value(&second.data).unwrap());
    let DesktopData::ChatRunMutation(started) = first.data else {
        panic!()
    };
    assert_eq!(started.run.status, ChatRunStatus::ModelUnconfigured);
    let messages = request(
        Some(identity.clone()),
        DesktopCommand::ChatMessagesList {
            conversation_id: conversation_id.clone(),
            after_ordinal: 0,
            limit: 20,
        },
    );
    let DesktopData::ChatMessages(items) = app.dispatch(messages.clone()).unwrap().data else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].content, "请解释为什么资料也可能有错。");
    assert!(items[1].content.is_empty());
    let page_request = request(
        Some(identity.clone()),
        DesktopCommand::ChatMessagesPage {
            conversation_id,
            before_ordinal: None,
            limit: 40,
        },
    );
    let page_wire = serde_json::to_value(&page_request).unwrap();
    let DesktopData::ChatMessagePage(page) = app
        .dispatch(DesktopRequest::parse(page_wire).unwrap())
        .unwrap()
        .data
    else {
        panic!()
    };
    assert_eq!(page.items.len(), 2);
    assert_eq!(
        page.latest_run.unwrap().last_event_seq,
        started.run.last_event_seq
    );
    assert!(page.next_before.is_none());
    let conversations_request = request(
        Some(identity.clone()),
        DesktopCommand::ChatConversationsPage {
            before: None,
            limit: 40,
        },
    );
    let DesktopData::ChatConversationPage(page) =
        app.dispatch(conversations_request.clone()).unwrap().data
    else {
        panic!()
    };
    assert_eq!(page.items.len(), 1);
    assert!(page.next_cursor.is_none());
    let events = app
        .dispatch(request(
            Some(identity),
            DesktopCommand::ChatRunsEvents {
                run_id: started.run.id,
                after_seq: 0,
                limit: 20,
            },
        ))
        .unwrap();
    let DesktopData::ChatRunEvents(events) = events.data else {
        panic!()
    };
    assert!(!events.is_empty());
    workspace(&mut app, &root.path().join("other-chat"), true);
    for page in [page_request, conversations_request] {
        assert_eq!(
            app.dispatch(page).unwrap_err().code,
            ApiErrorCode::InstanceMismatch
        );
    }
    assert_eq!(
        app.dispatch(messages).unwrap_err().code,
        ApiErrorCode::InstanceMismatch
    );
}

#[test]
fn global_annotation_candidates_use_live_workspace_and_public_wire_names() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("library"), true);
    let wire = json!({"api_version":1,"request_id":id(),"identity":identity,
        "command":{"kind":"annotations_query","payload":{"limit":20,"cursor":null,"query":"真实"}}});
    let parsed = DesktopRequest::parse(wire).unwrap();
    let reply = app.dispatch(parsed.clone()).unwrap();
    let DesktopData::AnnotationPage(page) = &reply.data else {
        panic!("wrong response")
    };
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
    assert_eq!(
        serde_json::to_value(reply).unwrap()["data"]["kind"],
        "annotation_page"
    );
    let _other = workspace(&mut app, &root.path().join("other"), true);
    assert_eq!(
        app.dispatch(parsed).unwrap_err().code,
        ApiErrorCode::InstanceMismatch
    );
}

#[test]
fn organization_commands_roundtrip_with_scoped_metadata_conflicts_and_receipts() {
    use pkms_core::{
        AssignTag, InboxState, MetadataTarget, NotesFilter, NotesPageRequest, RemoveTag,
        SetInboxState, TagSelector, TagsPageRequest,
    };
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("library"), true);
    let first = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::NotesCreate {
                command_id: id(),
                note: CreateNote {
                    collection_id: None,
                    base_collection_row_version: None,
                    title: "组织入口".into(),
                    document_schema: 2,
                    document: NoteDocument::from_text(id(), "真实正文"),
                },
            },
        ))
        .unwrap();
    let DesktopData::NoteCommit(first) = first.data else {
        panic!()
    };
    let get = DesktopCommand::ObjectsMetadataGet {
        object_id: first.note.id.clone(),
    };
    assert_eq!(
        app.dispatch(request(None, get.clone())).unwrap_err().code,
        ApiErrorCode::InstanceMismatch
    );
    let metadata = app
        .dispatch(request(Some(identity.clone()), get.clone()))
        .unwrap();
    let DesktopData::ObjectMetadata(metadata) = metadata.data else {
        panic!()
    };
    assert_eq!(metadata.row_version, 1);
    let assign = DesktopCommand::TagsAssign {
        command_id: id(),
        request: AssignTag {
            targets: vec![MetadataTarget {
                object_id: first.note.id.clone(),
                base_row_version: 1,
            }],
            tag: TagSelector::Name {
                name: "Rust".into(),
            },
        },
    };
    let parsed = DesktopRequest::parse(
        serde_json::to_value(request(Some(identity.clone()), assign.clone())).unwrap(),
    )
    .unwrap();
    let assigned = app.dispatch(parsed).unwrap();
    let DesktopData::MetadataCommit(assigned) = assigned.data else {
        panic!()
    };
    assert!(assigned.tag_created);
    let tag_id = assigned.tag.unwrap().id;
    let update = DesktopCommand::ObjectsSetInboxState {
        command_id: id(),
        request: SetInboxState {
            object_id: first.note.id.clone(),
            base_row_version: 1,
            target: InboxState::Processed,
        },
    };
    let error = app
        .dispatch(request(Some(identity.clone()), update))
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::MetadataConflict);
    assert!(!error.retryable);
    assert!(matches!(
        error.details,
        Some(ApiErrorDetails::MetadataConflict {
            expected: 1,
            actual: 2,
            ..
        })
    ));
    app.dispatch(request(
        Some(identity.clone()),
        DesktopCommand::ObjectsSetInboxState {
            command_id: id(),
            request: SetInboxState {
                object_id: first.note.id.clone(),
                base_row_version: 2,
                target: InboxState::Processed,
            },
        },
    ))
    .unwrap();
    let replay = app
        .dispatch(request(Some(identity.clone()), assign))
        .unwrap();
    let DesktopData::MetadataCommit(replay) = replay.data else {
        panic!()
    };
    assert!(replay.receipt.replayed);
    assert_eq!(replay.objects[0].inbox_state, Some(InboxState::Pending));
    let filtered = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::NotesList(NotesPageRequest {
                limit: 10,
                query: None,
                cursor: None,
                trashed: false,
                filter: Some(NotesFilter {
                    inbox_state: Some(InboxState::Processed),
                    tag_ids: vec![tag_id.clone()],
                    ..Default::default()
                }),
                sort: None,
            }),
        ))
        .unwrap();
    let DesktopData::Notes(filtered) = filtered.data else {
        panic!()
    };
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].row_version, 3);
    app.dispatch(request(
        Some(identity.clone()),
        DesktopCommand::TagsRemove {
            command_id: id(),
            request: RemoveTag {
                targets: vec![MetadataTarget {
                    object_id: first.note.id.clone(),
                    base_row_version: 3,
                }],
                tag_id,
            },
        },
    ))
    .unwrap();
    let listed = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::TagsList(TagsPageRequest {
                limit: 10,
                cursor: None,
                query: None,
            }),
        ))
        .unwrap();
    let DesktopData::Tags(listed) = listed.data else {
        panic!()
    };
    assert_eq!(listed.items.len(), 1);
    let note = app
        .dispatch(request(
            Some(identity),
            DesktopCommand::NotesGet {
                note_id: first.note.id,
            },
        ))
        .unwrap();
    let DesktopData::Note(note) = note.data else {
        panic!()
    };
    assert_eq!(note.revision_id, first.note.revision_id);
}

#[test]
fn envelope_rejects_unknown_fields_versions_and_unbounded_inputs() {
    let value = serde_json::to_value(request(None, DesktopCommand::AppState)).unwrap();
    assert!(DesktopRequest::parse(value.clone()).is_ok());
    let mut invalid = value.clone();
    invalid["surprise"] = json!(true);
    assert_eq!(
        DesktopRequest::parse(invalid).unwrap_err().code,
        ApiErrorCode::InvalidInput
    );
    let mut invalid = value.clone();
    invalid["command"]["extra"] = json!(true);
    assert!(DesktopRequest::parse(invalid).is_err());
    let mut invalid = value.clone();
    invalid["command"] = json!({"kind":"notes_list","payload":{"limit":1,"trashed":false,"cursor":null,"extra":true}});
    assert!(DesktopRequest::parse(invalid).is_err());
    let mut invalid = value.clone();
    invalid["api_version"] = json!(99);
    assert_eq!(
        DesktopRequest::parse(invalid).unwrap_err().code,
        ApiErrorCode::UnsupportedApi
    );
    let mut invalid = value;
    invalid["request_id"] = json!("not a uuid");
    assert!(DesktopRequest::parse(invalid).is_err());
    let error = ApiError::from_app(
        AppError::Io(std::io::Error::other(
            "SECRET-CONTENT C:\\private\\secret.txt",
        )),
        &id(),
    );
    let encoded = serde_json::to_string(&error).unwrap();
    assert!(!encoded.contains("SECRET") && !encoded.contains("private"));
}

#[test]
fn collection_and_saved_view_commands_share_workspace_scope_and_query_real_members() {
    use pkms_core::{
        ChangeCollectionMembers, CollectionMembersRequest, CreateCollection, NoteSort, NotesFilter,
        SaveView, SetObjectFlag, ViewConfig, ViewLayout,
    };
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let note = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::NotesCreate {
                command_id: id(),
                note: CreateNote {
                    collection_id: None,
                    base_collection_row_version: None,
                    title: "成员".into(),
                    document_schema: 2,
                    document: NoteDocument::from_text(id(), "正文"),
                },
            },
        ))
        .unwrap();
    let DesktopData::NoteCommit(note) = note.data else {
        panic!()
    };
    let create = DesktopCommand::CollectionsCreate {
        command_id: id(),
        request: CreateCollection {
            name: "主题".into(),
            overview_note_id: Some(note.note.id.clone()),
            is_pinned: true,
            pin_position: Some(0),
        },
    };
    assert_eq!(
        app.dispatch(request(None, create.clone()))
            .unwrap_err()
            .code,
        ApiErrorCode::InstanceMismatch
    );
    let envelope = DesktopRequest::parse(
        serde_json::to_value(request(Some(identity.clone()), create)).unwrap(),
    )
    .unwrap();
    let collection = app.dispatch(envelope).unwrap();
    let DesktopData::CollectionCommit(collection) = collection.data else {
        panic!()
    };
    let collection_id = collection.collection.id;
    app.dispatch(request(
        Some(identity.clone()),
        DesktopCommand::CollectionsAddMembers {
            command_id: id(),
            request: ChangeCollectionMembers {
                collection_id: collection_id.clone(),
                base_row_version: 1,
                member_ids: vec![note.note.id.clone()],
            },
        },
    ))
    .unwrap();
    let view = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::ViewsSave {
                command_id: id(),
                request: SaveView {
                    view_id: None,
                    base_row_version: None,
                    name: "主题视图".into(),
                    filter_schema: 1,
                    filter: NotesFilter {
                        collection_id: Some(collection_id.clone()),
                        ..Default::default()
                    },
                    sort: NoteSort::TitleAsc,
                    layout: ViewLayout::Table,
                    config_schema: 1,
                    config: ViewConfig::default(),
                    is_pinned: false,
                    pin_position: None,
                },
            },
        ))
        .unwrap();
    let DesktopData::ViewCommit(view) = view.data else {
        panic!()
    };
    let run = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::ViewsRun {
                view_id: view.view.id,
                limit: 100,
                cursor: None,
            },
        ))
        .unwrap();
    let DesktopData::ViewRun(run) = run.data else {
        panic!()
    };
    assert_eq!(run.notes.items[0].note.id, note.note.id);
    app.dispatch(request(
        Some(identity.clone()),
        DesktopCommand::ObjectsSetTrashed {
            command_id: id(),
            request: SetObjectFlag {
                object_id: collection_id.clone(),
                base_row_version: 2,
                value: true,
            },
        },
    ))
    .unwrap();
    let members = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::CollectionsMembers(CollectionMembersRequest {
                collection_id,
                limit: 100,
                cursor: None,
                include_trashed: false,
                kind: None,
            }),
        ))
        .unwrap();
    let DesktopData::CollectionMembers(members) = members.data else {
        panic!()
    };
    assert_eq!(members.items[0].object.id, note.note.id);
    assert!(!members.items[0].object.trashed);
    let stale = WorkspaceIdentity {
        local_instance_id: id(),
        ..identity
    };
    let error = app
        .dispatch(request(
            Some(stale),
            DesktopCommand::ViewsGet {
                view_id: run.view.id,
            },
        ))
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::InstanceMismatch);
}

#[test]
fn stale_close_and_note_commands_cannot_cross_workspace_instances() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let a = workspace(&mut app, &root.path().join("a"), true);
    let b = workspace(&mut app, &root.path().join("b"), true);
    for identity in [None, Some(a)] {
        let error = app
            .dispatch(request(identity, DesktopCommand::WorkspaceClose))
            .unwrap_err();
        assert_eq!(error.code, ApiErrorCode::InstanceMismatch);
        assert_eq!(app.active_workspace().unwrap().identity, b);
    }
    let command = id();
    let create = DesktopCommand::NotesCreate {
        command_id: command.clone(),
        note: CreateNote {
            collection_id: None,
            base_collection_row_version: None,
            title: "真实笔记".into(),
            document_schema: 2,
            document: NoteDocument::from_text(id(), "保留正文"),
        },
    };
    let first = app
        .dispatch(request(Some(b.clone()), create.clone()))
        .unwrap();
    let DesktopData::NoteCommit(first) = first.data else {
        panic!()
    };
    let repeat = app.dispatch(request(Some(b.clone()), create)).unwrap();
    let DesktopData::NoteCommit(repeat) = repeat.data else {
        panic!()
    };
    assert!(repeat.receipt.replayed);
    assert_eq!(repeat.note, first.note);
    let save = SaveNote {
        note_id: first.note.id,
        base_revision_id: first.note.revision_id,
        title: "更新".into(),
        document_schema: 2,
        document: first.note.document,
    };
    app.dispatch(request(
        Some(b.clone()),
        DesktopCommand::NotesSave {
            command_id: id(),
            note: save.clone(),
        },
    ))
    .unwrap();
    let error = app
        .dispatch(request(
            Some(b.clone()),
            DesktopCommand::NotesSave {
                command_id: id(),
                note: save,
            },
        ))
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::RevisionConflict);
    assert!(error.details.is_some());
    app.dispatch(request(Some(b), DesktopCommand::WorkspaceClose))
        .unwrap();
    assert!(app.active_workspace().is_none());
}

#[test]
fn attachment_receipt_survives_input_loss_expiry_and_restart_without_scope_leak() {
    let root = tempdir().unwrap();
    let app_dir = root.path().join("app");
    let library_dir = root.path().join("knowledge");
    let input = root.path().join("image.png");
    std::fs::write(&input, picture()).unwrap();
    let mut app = AppService::open(&app_dir).unwrap();
    let identity = workspace(&mut app, &library_dir, true);
    let token = app
        .grant_path(GrantPurpose::ImportFile, &input)
        .unwrap()
        .token;
    let command_id = id();
    let result = app
        .import_selected_attachment(&identity, &command_id, &token)
        .unwrap();
    std::fs::remove_file(&input).unwrap();
    Connection::open(app_dir.join("app-state.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE local_resource_grants SET expires_at=0 WHERE token=?1",
            [&token],
        )
        .unwrap();
    let replay = app
        .import_selected_attachment(&identity, &command_id, &token)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.blob, result.blob);
    assert!(matches!(
        app.import_selected_attachment(&identity, &command_id, &id()),
        Err(AppError::IdempotencyMismatch)
    ));
    drop(app);
    let mut app = AppService::open(&app_dir).unwrap();
    let reopened = workspace(&mut app, &library_dir, false);
    assert_eq!(reopened, identity);
    assert!(
        app.library(&identity)
            .unwrap()
            .attachment_preview(&result.preview_token)
            .is_err()
    );
    let resumed = app
        .import_selected_attachment(&identity, &command_id, &token)
        .unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .attachment_preview(&resumed.preview_token)
            .unwrap(),
        picture()
    );
    let other = workspace(&mut app, &root.path().join("other"), true);
    assert!(matches!(
        app.import_selected_attachment(&identity, &command_id, &token),
        Err(AppError::InstanceMismatch)
    ));
    assert!(
        app.import_selected_attachment(&other, &command_id, &token)
            .is_err()
    );
    assert!(
        app.library_mut(&other)
            .unwrap()
            .resume_attachment_import(&command_id)
            .is_err()
    );
}

#[test]
fn an_existing_core_command_cannot_turn_a_mismatched_native_import_into_success_on_retry() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let command = id();
    app.library_mut(&identity)
        .unwrap()
        .import_attachment(&command, "original.png", &picture())
        .unwrap();
    let path = root.path().join("different-name.png");
    std::fs::write(&path, picture()).unwrap();
    let token = app
        .grant_path(GrantPurpose::ImportFile, path)
        .unwrap()
        .token;
    for _ in 0..2 {
        let error = app
            .import_selected_attachment(&identity, &command, &token)
            .unwrap_err();
        assert_eq!(
            ApiError::from_app(error, &id()).code,
            ApiErrorCode::IdempotencyMismatch
        );
    }
}

#[test]
fn old_language_preferences_default_to_chinese_and_persist_valid_updates() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path()).unwrap();
    let original = app.preferences().unwrap();
    let mut legacy = serde_json::to_value(&original.preferences).unwrap();
    legacy.as_object_mut().unwrap().remove("language");
    let raw = Connection::open(root.path().join("app-state.sqlite3")).unwrap();
    let schema: i64 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    raw.execute(
        "UPDATE app_preferences SET preferences_json=?1",
        [legacy.to_string()],
    )
    .unwrap();
    assert_eq!(app.preferences().unwrap(), original);
    let stored: String = raw
        .query_row("SELECT preferences_json FROM app_preferences", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stored).unwrap(),
        legacy
    );

    let mut preferences = original.preferences;
    preferences.language = Language::English;
    let envelope = request(
        None,
        DesktopCommand::PreferencesUpdate {
            base_state_version: original.state_version,
            preferences,
        },
    );
    let value = serde_json::to_value(envelope).unwrap();
    let valid = DesktopRequest::parse(value.clone()).unwrap();
    let DesktopData::Preferences(saved) = app.dispatch(valid).unwrap().data else {
        panic!()
    };
    assert_eq!(saved.preferences.language, Language::English);
    let mut invalid = value;
    invalid["command"]["payload"]["base_state_version"] = json!(saved.state_version);
    invalid["command"]["payload"]["preferences"]["language"] = json!("not-a-supported-language");
    assert_eq!(
        DesktopRequest::parse(invalid).unwrap_err().code,
        ApiErrorCode::InvalidInput
    );
    assert_eq!(app.preferences().unwrap(), saved);
    assert_eq!(
        raw.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        schema
    );
    drop(app);
    assert_eq!(
        AppService::open(root.path())
            .unwrap()
            .preferences()
            .unwrap(),
        saved
    );
}

#[test]
fn app_v1_upgrade_keeps_preferences_and_independent_snapshot() {
    let root = tempdir().unwrap();
    let connection = Connection::open(root.path().join("app-state.sqlite3")).unwrap();
    connection
        .execute_batch(include_str!("../src/schema.sql"))
        .unwrap();
    let preferences = AppPreferences {
        theme: Theme::Dark,
        ..Default::default()
    };
    connection
        .execute(
            "INSERT INTO app_preferences VALUES(1,7,?1)",
            [serde_json::to_string(&preferences).unwrap()],
        )
        .unwrap();
    drop(connection);
    let app = AppService::open(root.path()).unwrap();
    assert_eq!(app.preferences().unwrap().state_version, 7);
    assert_eq!(app.preferences().unwrap().preferences, preferences);
    let backup = std::fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("app-state-v1-")
        })
        .unwrap();
    let snapshot = Connection::open(backup).unwrap();
    assert_eq!(
        snapshot
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        snapshot
            .query_row("SELECT state_version FROM app_preferences", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        7
    );
}

#[test]
fn hiding_a_recent_workspace_keeps_its_identity_files_and_reopen_entry() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let first = workspace(&mut app, &root.path().join("first"), true);
    let second = workspace(&mut app, &root.path().join("second"), true);
    let listed = recent_identities(&mut app);
    assert!(listed.contains(&first) && listed.contains(&second));
    // The welcome page only offers this while no workspace is open.
    app.dispatch(request(
        Some(second.clone()),
        DesktopCommand::WorkspaceClose,
    ))
    .unwrap();
    let forgotten = app
        .dispatch(request(
            Some(first.clone()),
            DesktopCommand::WorkspaceForget,
        ))
        .unwrap();
    assert!(matches!(forgotten.data, DesktopData::Done));
    let listed = recent_identities(&mut app);
    assert!(!listed.contains(&first) && listed.contains(&second));
    // Hiding an entry never touches the workspace on disk.
    assert!(root.path().join("first").exists());
    // Opening it again restores the recent entry with the same identity.
    assert_eq!(
        workspace(&mut app, &root.path().join("first"), false),
        first
    );
    let listed = recent_identities(&mut app);
    assert!(listed.contains(&first) && listed.contains(&second));
}

#[test]
fn copying_a_note_over_ipc_creates_an_independent_note_and_replays_once() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("copy"), true);
    let DesktopData::NoteCommit(source) = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::NotesCreate {
                command_id: id(),
                note: CreateNote {
                    collection_id: None,
                    base_collection_row_version: None,
                    title: "会议记录".into(),
                    document_schema: 2,
                    document: NoteDocument::from_text(id(), "第一段正文"),
                },
            },
        ))
        .unwrap()
        .data
    else {
        panic!()
    };
    let copy = request(
        Some(identity.clone()),
        DesktopCommand::NotesCopy {
            command_id: id(),
            note: CopyNote {
                note_id: source.note.id.clone(),
                source_revision_id: source.note.revision_id.clone(),
                title: "会议记录 副本".into(),
            },
        },
    );
    let DesktopData::NoteCommit(copied) = app.dispatch(copy.clone()).unwrap().data else {
        panic!()
    };
    assert_ne!(copied.note.id, source.note.id);
    assert_eq!(copied.note.title, "会议记录 副本");
    assert_eq!(copied.note.revision, 1);
    assert_eq!(copied.receipt.object_id, copied.note.id);
    // The copy is a real new document: same text, remapped internal identifiers.
    assert_eq!(
        copied.note.document.plain_text(),
        source.note.document.plain_text()
    );
    assert_ne!(
        serde_json::to_value(&copied.note.document).unwrap(),
        serde_json::to_value(&source.note.document).unwrap()
    );
    // A retried copy is idempotent and never creates a second note.
    let DesktopData::NoteCommit(replayed) = app.dispatch(copy).unwrap().data else {
        panic!()
    };
    assert_eq!(replayed.note.id, copied.note.id);
    assert!(replayed.receipt.replayed);
    // The source revision is untouched.
    let DesktopData::Note(original) = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::NotesGet {
                note_id: source.note.id.clone(),
            },
        ))
        .unwrap()
        .data
    else {
        panic!()
    };
    assert_eq!(original.revision_id, source.note.revision_id);
    assert_eq!(original.title, "会议记录");
    let DesktopData::Notes(notes) = app
        .dispatch(request(
            Some(identity),
            DesktopCommand::NotesList(pkms_core::NotesPageRequest {
                limit: 20,
                query: None,
                cursor: None,
                trashed: false,
                filter: None,
                sort: None,
            }),
        ))
        .unwrap()
        .data
    else {
        panic!()
    };
    assert_eq!(notes.items.len(), 2);
}

#[test]
fn opening_an_object_fills_the_recent_list_and_purging_it_drops_only_that_row() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("recent"), true);
    let first = create_note(&mut app, &identity, "第一");
    let second = create_note(&mut app, &identity, "第二");
    assert!(recent_object_ids(&mut app, &identity).is_empty());
    // The list only exists because opening an object records it.
    app.dispatch(request(
        Some(identity.clone()),
        DesktopCommand::RecentObjectsTouch {
            object_id: first.clone(),
        },
    ))
    .unwrap();
    assert_eq!(recent_object_ids(&mut app, &identity), vec![first.clone()]);
    app.dispatch(request(
        Some(identity.clone()),
        DesktopCommand::RecentObjectsTouch {
            object_id: second.clone(),
        },
    ))
    .unwrap();
    assert_eq!(
        recent_object_ids(&mut app, &identity),
        vec![second.clone(), first.clone()]
    );
    // Re-opening the older object moves it back to the front.
    app.dispatch(request(
        Some(identity.clone()),
        DesktopCommand::RecentObjectsTouch {
            object_id: first.clone(),
        },
    ))
    .unwrap();
    assert_eq!(
        recent_object_ids(&mut app, &identity),
        vec![first.clone(), second.clone()]
    );
    // Unknown identifiers are rejected instead of polluting the list.
    assert_eq!(
        app.dispatch(request(
            Some(identity.clone()),
            DesktopCommand::RecentObjectsTouch { object_id: id() },
        ))
        .unwrap_err()
        .code,
        ApiErrorCode::NotFound
    );
    assert_eq!(recent_object_ids(&mut app, &identity).len(), 2);
    // A trashed object keeps its row (the inbox marks the state)...
    let DesktopData::ObjectMetadata(metadata) = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::ObjectsMetadataGet {
                object_id: second.clone(),
            },
        ))
        .unwrap()
        .data
    else {
        panic!()
    };
    app.dispatch(request(
        Some(identity.clone()),
        DesktopCommand::ObjectsSetTrashed {
            command_id: id(),
            request: SetObjectFlag {
                object_id: second.clone(),
                base_row_version: metadata.row_version,
                value: true,
            },
        },
    ))
    .unwrap();
    assert_eq!(recent_object_ids(&mut app, &identity).len(), 2);
    // ...and a permanent delete removes exactly that row.
    let DesktopData::TrashPreview(snapshot) = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::TrashPreview(TrashPreviewRequest {
                selection: TrashSelection::Objects {
                    object_ids: vec![second.clone()],
                },
            }),
        ))
        .unwrap()
        .data
    else {
        panic!()
    };
    app.dispatch(request(
        Some(identity.clone()),
        DesktopCommand::TrashDelete {
            command_id: id(),
            request: DeleteTrash {
                selection: snapshot.selection,
                snapshot_token: snapshot.snapshot_token,
            },
        },
    ))
    .unwrap();
    assert_eq!(recent_object_ids(&mut app, &identity), vec![first.clone()]);
}

/// The property backlink command is reachable over its public wire name and
/// keeps the bounded, generation-bound cursor semantics of the reference pages.
#[test]
fn relation_backlink_command_uses_the_public_wire_name_and_rejects_bad_input() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("library"), true);
    let target = create_note(&mut app, &identity, "Target");
    let document: NoteDocument = serde_json::from_value(json!({
        "type": "doc",
        "content": [
            { "type": "frontmatter", "attrs": { "id": id(), "source": format!("related:\n  - kind: note\n    id: {target}\n    title: Target\n") } },
            { "type": "paragraph", "attrs": { "id": id() } }
        ]
    }))
    .unwrap();
    let DesktopData::NoteCommit(source) = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::NotesCreate {
                command_id: id(),
                note: CreateNote {
                    collection_id: None,
                    base_collection_row_version: None,
                    title: "Source".into(),
                    document_schema: 3,
                    document,
                },
            },
        ))
        .unwrap()
        .data
    else {
        panic!()
    };

    let wire = json!({"api_version":1,"request_id":id(),"identity":identity,
        "command":{"kind":"notes_relation_backlinks","payload":{"id":target,"limit":20,"cursor":null}}});
    let reply = app.dispatch(DesktopRequest::parse(wire).unwrap()).unwrap();
    let DesktopData::RelationBacklinkPage(page) = &reply.data else {
        panic!("wrong response")
    };
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].source_id, source.note.id);
    assert_eq!(page.items[0].source_revision_id, source.note.revision_id);
    assert_eq!(page.items[0].source_title, "Source");
    assert_eq!(page.items[0].field_key, "related");
    assert_eq!(page.items[0].relation_title, "Target");
    assert!(!page.items[0].source_trashed);
    assert_eq!(page.scan_limit, 10_000);
    assert!(!page.truncated);
    assert!(page.next_cursor.is_none());

    // A malformed cursor and a non-UUID target stay ordinary input errors.
    let error = app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::NotesRelationBacklinks {
                id: target.clone(),
                limit: 20,
                cursor: Some("not-json".into()),
            },
        ))
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::InvalidInput);
    let error = app
        .dispatch(request(
            Some(identity),
            DesktopCommand::NotesRelationBacklinks {
                id: "not-a-uuid".into(),
                limit: 20,
                cursor: None,
            },
        ))
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::InvalidInput);
}

#[test]
fn voice_capture_commands_round_trip_audio_and_promote_a_note() {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    let directory = tempdir().unwrap();
    let mut app = AppService::open(directory.path().join("app")).unwrap();
    let identity = workspace(&mut app, &directory.path().join("workspace"), true);
    let audio = b"\x1a\x45\xdf\xa3command-audio";
    let capture = match app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::VoiceCaptureCreate {
                command_id: id(),
                request: pkms_core::CreateVoiceCapture {
                    title: String::new(),
                    duration_ms: 2_000,
                    source_kind: pkms_core::VoiceSourceKind::Local,
                    device_label: None,
                    language: Some("zh".into()),
                },
                audio_base64: BASE64.encode(audio),
            },
        ))
        .unwrap()
        .data
    {
        DesktopData::VoiceCapture(capture) => capture,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(capture.status, pkms_core::VoiceCaptureStatus::Pending);
    assert_eq!(capture.audio_bytes, audio.len() as u32);

    let page = match app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::VoiceCaptures { trashed: false },
        ))
        .unwrap()
        .data
    {
        DesktopData::VoiceCaptures(page) => page,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, capture.id);

    let ready = match app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::VoiceCaptureUpdate {
                request: pkms_core::UpdateVoiceCapture {
                    capture_id: capture.id.clone(),
                    base_row_version: capture.row_version,
                    title: "口述灵感".into(),
                    transcript: "第一行\n第二行".into(),
                },
            },
        ))
        .unwrap()
        .data
    {
        DesktopData::VoiceCapture(capture) => capture,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(ready.status, pkms_core::VoiceCaptureStatus::Ready);
    assert_eq!(
        ready.transcript_source,
        pkms_core::VoiceTranscriptSource::Manual
    );

    let promoted = match app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::VoiceCaptureSaveNote {
                command_id: id(),
                request: pkms_core::SaveVoiceCaptureNote {
                    capture_id: capture.id.clone(),
                    base_row_version: ready.row_version,
                    collection_id: None,
                    base_collection_row_version: None,
                    tag_names: vec!["语音".into()],
                },
            },
        ))
        .unwrap()
        .data
    {
        DesktopData::VoiceCapture(capture) => capture,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(promoted.note_id.as_deref(), Some(capture.id.as_str()));
    assert!(app.library(&identity).unwrap().note(&capture.id).is_ok());
    let metadata = app
        .library(&identity)
        .unwrap()
        .object_metadata(&capture.id)
        .unwrap();
    assert!(metadata.tags.iter().any(|tag| tag.display_name == "语音"));

    let settings = match app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::AsrSettingsGet,
        ))
        .unwrap()
        .data
    {
        DesktopData::AsrSettings(settings) => settings,
        other => panic!("unexpected {other:?}"),
    };
    assert!(settings.settings.endpoint.is_empty());
    assert!(!settings.credential_configured);
    let saved = match app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::AsrSettingsSave(AsrSettingsSave {
                request: pkms_core::UpdateAsrSettings {
                    base_row_version: settings.settings.row_version,
                    endpoint: "https://api.example.test/v1".into(),
                    model: "whisper-1".into(),
                    language: "zh".into(),
                },
                credential: None,
                clear_credential: false,
            }),
        ))
        .unwrap()
        .data
    {
        DesktopData::AsrSettings(settings) => settings,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(saved.settings.model, "whisper-1");
    assert!(!saved.credential_configured);

    let trashed = match app
        .dispatch(request(
            Some(identity.clone()),
            DesktopCommand::VoiceCaptureTrash {
                request: pkms_core::VoiceCaptureTrashRequest {
                    capture_id: capture.id.clone(),
                    base_row_version: promoted.row_version,
                    trashed: true,
                },
            },
        ))
        .unwrap()
        .data
    {
        DesktopData::VoiceCapture(capture) => capture,
        other => panic!("unexpected {other:?}"),
    };
    assert!(trashed.trashed);
    assert!(
        app.dispatch(request(
            Some(identity),
            DesktopCommand::VoiceCaptureGet {
                capture_id: capture.id,
            },
        ))
        .is_ok()
    );
}

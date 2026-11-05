//! Cross-machine sync drill helper (Windows host <-> Linux container).
//!
//! This is verification tooling, not a product entry point. It splits the
//! provisioning and sync steps into one-shot commands so each can run on a
//! different machine while talking to the same authority over HTTP.

use pkms_core::{
    BoardGroupBy, CanvasNode, CreateNote, DOCUMENT_SCHEMA, Library, NoteDocument, NoteSort,
    NotesFilter, RemoveView, SaveCanvas, SaveNote, SaveView, ViewConfig, ViewLayout, ViewProperty,
};
use pkms_sync::{
    ConflictRecord, DeviceKeyPair, WorkspaceKey, owner_proof_message, parse_encryption_public_key,
    workspace_key_aad, wrap_workspace_key,
};
use pkms_sync_client::SyncClient;
use pkms_sync_engine::SyncEngine;
use serde_json::json;
use uuid::Uuid;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|value| value == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn req(args: &[String], name: &str) -> String {
    arg(args, name).unwrap_or_else(|| panic!("missing required argument {name}"))
}

fn bytes32(hex: &str) -> [u8; 32] {
    let raw = hex_encode_decode(hex.trim());
    assert_eq!(raw.len(), 32, "expected 32 bytes of hex");
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    out
}

fn hex_str(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn hex_encode_decode(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "hex must have an even length");
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let pair = std::str::from_utf8(&bytes[index..index + 2]).unwrap();
        out.push(u8::from_str_radix(pair, 16).expect("valid hex"));
        index += 2;
    }
    out
}

fn keypair(args: &[String]) -> DeviceKeyPair {
    DeviceKeyPair::from_seed(bytes32(&req(args, "--seed"))).expect("device key pair")
}

fn device_id(args: &[String]) -> String {
    req(args, "--device-id")
}

fn client_with(args: &[String], id: &str) -> SyncClient {
    SyncClient::new(req(args, "--url"), id, keypair(args)).expect("sync client")
}

fn client(args: &[String]) -> SyncClient {
    client_with(args, &device_id(args))
}

fn load_wsk(args: &[String]) -> WorkspaceKey {
    let hex = std::fs::read_to_string(req(args, "--wsk")).expect("read workspace key file");
    WorkspaceKey::from_bytes(bytes32(&hex))
}

fn epoch(args: &[String]) -> i64 {
    req(args, "--epoch").parse().expect("epoch")
}

fn emit(value: serde_json::Value) {
    println!("{value}");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().cloned().unwrap_or_default();
    let rest = &args[1..];
    match command.as_str() {
        "init" => {
            let library = Library::open(req(rest, "--library")).expect("open library");
            let info = library.info().expect("library info");
            emit(json!({"workspace_id": info.workspace_id, "schema": info.database_schema}));
        }
        "keys" => {
            let keys = keypair(rest);
            let name = req(rest, "--name");
            let proof = keys.sign(&owner_proof_message(&keys.public_key_hex(), &name));
            emit(json!({
                "public_key": keys.public_key_hex(),
                "encryption_public_key": keys.encryption_public_key_hex().unwrap(),
                "proof": hex_str(&proof),
                "display_name": name,
            }));
        }
        "invite" => {
            let invite = client(rest).create_pairing_invite(900).expect("invite");
            emit(json!({"invite_id": invite.invite_id, "code": invite.code}));
        }
        "join" => {
            let record = client_with(rest, "pairing")
                .pair_with_code(
                    &req(rest, "--invite-id"),
                    &req(rest, "--code"),
                    &req(rest, "--name"),
                )
                .expect("pair");
            emit(json!({
                "device_id": record.device_id,
                "display_name": record.display_name,
            }));
        }
        "register-key" => {
            let keys = keypair(rest);
            client(rest)
                .register_encryption_key(&keys.encryption_public_key_hex().unwrap())
                .expect("register encryption key");
            emit(json!({"registered": device_id(rest)}));
        }
        "rotate" => {
            let client = client(rest);
            let workspace_id = req(rest, "--workspace-id");
            let next_epoch = epoch(rest);
            let key = WorkspaceKey::generate().expect("workspace key");
            let keys = client.device_encryption_keys().expect("device keys");
            let mut envelopes = Vec::new();
            for entry in &keys {
                let public = parse_encryption_public_key(&entry.public_key).expect("public key");
                envelopes.push((
                    entry.device_id.clone(),
                    wrap_workspace_key(
                        public,
                        &key,
                        &workspace_key_aad(&workspace_id, next_epoch, &entry.device_id),
                    )
                    .expect("wrap"),
                ));
            }
            let stored = client
                .rotate_workspace_key(&workspace_id, next_epoch, &envelopes)
                .expect("rotate");
            std::fs::write(req(rest, "--wsk-out"), key.export_hex()).expect("write wsk");
            emit(json!({"epoch": stored, "devices": keys.len()}));
        }
        "unwrap" => {
            let client = client(rest);
            let workspace_id = req(rest, "--workspace-id");
            let key_epoch = epoch(rest);
            let envelope = client
                .fetch_key_envelope(&workspace_id, key_epoch)
                .expect("key envelope");
            let keys = keypair(rest);
            let key = keys
                .unwrap_workspace_key(
                    &envelope,
                    &workspace_key_aad(&workspace_id, key_epoch, &device_id(rest)),
                )
                .expect("unwrap workspace key");
            std::fs::write(req(rest, "--wsk-out"), key.export_hex()).expect("write wsk");
            emit(json!({"epoch": key_epoch, "unwrapped": true}));
        }
        "push" => {
            let mut library = Library::open(req(rest, "--library")).expect("open library");
            let title = req(rest, "--title");
            let body = match arg(rest, "--body-repeat") {
                Some(count) => "磁盘满演练正文。".repeat(count.parse().expect("repeat count")),
                None => req(rest, "--body"),
            };
            let note_id = match req(rest, "--note").as_str() {
                "new" => {
                    let note = library
                        .create_document(
                            &Uuid::new_v4().to_string(),
                            CreateNote {
                                title,
                                document_schema: DOCUMENT_SCHEMA,
                                document: NoteDocument::from_text(
                                    Uuid::new_v4().to_string(),
                                    &body,
                                ),
                                collection_id: None,
                                base_collection_row_version: None,
                            },
                        )
                        .expect("create note")
                        .note;
                    note.id
                }
                existing => {
                    let note = library.note(existing).expect("load note");
                    library
                        .save_document(
                            &Uuid::new_v4().to_string(),
                            SaveNote {
                                note_id: note.id.clone(),
                                base_revision_id: note.revision_id.clone(),
                                title,
                                document_schema: note.schema_version,
                                document: NoteDocument::from_text(
                                    Uuid::new_v4().to_string(),
                                    &body,
                                ),
                            },
                        )
                        .expect("save note")
                        .note
                        .id
                }
            };
            let mut engine = SyncEngine::with_state(
                client(rest),
                load_wsk(rest),
                req(rest, "--workspace-id"),
                epoch(rest),
                req(rest, "--state"),
            )
            .expect("engine");
            let outcome = engine.push_note(&library, &note_id).expect("push");
            emit(json!({"note": note_id, "outcome": outcome}));
        }
        "pull" => {
            let mut library = Library::open(req(rest, "--library")).expect("open library");
            let mut engine = SyncEngine::with_state(
                client(rest),
                load_wsk(rest),
                req(rest, "--workspace-id"),
                epoch(rest),
                req(rest, "--state"),
            )
            .expect("engine");
            let summary = engine.pull_and_apply(&mut library, 100).expect("pull");
            let notes: Vec<_> = library
                .list_notes()
                .expect("notes")
                .into_iter()
                .map(|note| {
                    json!({
                        "id": note.id,
                        "title": note.title,
                        "body": note.document.plain_text(),
                        "trashed": note.trashed,
                        "revision": note.revision_id,
                    })
                })
                .collect();
            let views: Vec<_> = library
                .list_views()
                .expect("views")
                .into_iter()
                .map(|view| {
                    json!({
                        "id": view.id,
                        "name": view.name,
                        "layout": format!("{:?}", view.layout),
                        "row_version": view.row_version,
                        "board_group_by": format!("{:?}", view.config.board_group_by),
                    })
                })
                .collect();
            let canvases: Vec<_> = library
                .list_canvases()
                .expect("canvases")
                .into_iter()
                .map(|canvas| {
                    let view = library.canvas_get(&canvas.id).expect("canvas view");
                    json!({
                        "id": canvas.id,
                        "name": canvas.name,
                        "nodes": canvas.nodes.len(),
                        "notes": view.notes.len(),
                        "missing": view.missing_note_ids,
                        "row_version": canvas.row_version,
                    })
                })
                .collect();
            emit(json!({
                "applied": summary.applied,
                "skipped": summary.skipped,
                "conflicts": summary.conflicts,
                "server_conflicts": summary.server_conflicts,
                "config_conflicts": summary.config_conflicts,
                "next_cursor": summary.next_cursor,
                "notes": notes,
                "views": views,
                "canvases": canvases,
            }));
        }
        "create-view" => {
            let mut library = Library::open(req(rest, "--library")).expect("open library");
            let view = library
                .view_save(
                    &Uuid::new_v4().to_string(),
                    SaveView {
                        view_id: None,
                        base_row_version: None,
                        name: req(rest, "--name"),
                        filter_schema: 1,
                        filter: NotesFilter {
                            inbox_state: None,
                            tag_ids: vec![],
                            collection_id: None,
                            archived: Some(false),
                            search_enabled: None,
                            object_types: vec![],
                        },
                        sort: NoteSort::UpdatedDesc,
                        layout: ViewLayout::Board,
                        config_schema: 1,
                        config: ViewConfig {
                            visible_properties: vec![
                                ViewProperty::InboxState,
                                ViewProperty::Tags,
                                ViewProperty::UpdatedAt,
                            ],
                            board_group_by: BoardGroupBy::InboxState,
                            calendar_month: None,
                            ..Default::default()
                        },
                        is_pinned: false,
                        pin_position: None,
                    },
                )
                .expect("view save")
                .view;
            emit(json!({"view": view.id, "name": view.name, "row_version": view.row_version}));
        }
        "edit-view" => {
            let mut library = Library::open(req(rest, "--library")).expect("open library");
            let current = library.view_get(&req(rest, "--view")).expect("view");
            let saved = library
                .view_save(
                    &Uuid::new_v4().to_string(),
                    SaveView {
                        view_id: Some(current.id.clone()),
                        base_row_version: Some(current.row_version),
                        name: req(rest, "--name"),
                        filter_schema: current.filter_schema,
                        filter: current.filter.clone(),
                        sort: current.sort,
                        layout: current.layout,
                        config_schema: current.config_schema,
                        config: current.config.clone(),
                        is_pinned: current.is_pinned,
                        pin_position: current.pin_position,
                    },
                )
                .expect("view save")
                .view;
            emit(json!({"view": saved.id, "name": saved.name, "row_version": saved.row_version}));
        }
        "push-view" => {
            let library = Library::open(req(rest, "--library")).expect("open library");
            let mut engine = SyncEngine::with_state(
                client(rest),
                load_wsk(rest),
                req(rest, "--workspace-id"),
                epoch(rest),
                req(rest, "--state"),
            )
            .expect("engine");
            let view_id = req(rest, "--view");
            let outcome = engine.push_view(&library, &view_id).expect("push view");
            emit(json!({"view": view_id, "outcome": outcome}));
        }
        "remove-view" => {
            let mut library = Library::open(req(rest, "--library")).expect("open library");
            let view_id = req(rest, "--view");
            let current = library.view_get(&view_id).expect("view");
            library
                .view_remove(
                    &Uuid::new_v4().to_string(),
                    RemoveView {
                        view_id: current.id.clone(),
                        base_row_version: current.row_version,
                    },
                )
                .expect("remove view");
            let mut engine = SyncEngine::with_state(
                client(rest),
                load_wsk(rest),
                req(rest, "--workspace-id"),
                epoch(rest),
                req(rest, "--state"),
            )
            .expect("engine");
            let outcome = engine.push_view_remove(&view_id).expect("push view remove");
            emit(json!({"view": view_id, "outcome": outcome}));
        }
        "create-canvas" => {
            let mut library = Library::open(req(rest, "--library")).expect("open library");
            let note = library
                .create_document(
                    &Uuid::new_v4().to_string(),
                    CreateNote {
                        title: req(rest, "--title"),
                        document_schema: DOCUMENT_SCHEMA,
                        document: NoteDocument::from_text(
                            Uuid::new_v4().to_string(),
                            &req(rest, "--body"),
                        ),
                        collection_id: None,
                        base_collection_row_version: None,
                    },
                )
                .expect("create note")
                .note;
            let canvas = library
                .canvas_save(
                    &Uuid::new_v4().to_string(),
                    SaveCanvas {
                        canvas_id: None,
                        base_row_version: None,
                        name: req(rest, "--name"),
                        config_schema: 1,
                        nodes: vec![CanvasNode {
                            note_id: note.id.clone(),
                            x: 40.0,
                            y: 60.0,
                            width: 250.0,
                            height: 148.0,
                            z_index: 0,
                        }],
                        is_pinned: false,
                        pin_position: None,
                    },
                )
                .expect("canvas save")
                .canvas;
            emit(json!({"canvas": canvas.id, "note": note.id, "nodes": canvas.nodes.len()}));
        }
        "push-note" => {
            let library = Library::open(req(rest, "--library")).expect("open library");
            let mut engine = SyncEngine::with_state(
                client(rest),
                load_wsk(rest),
                req(rest, "--workspace-id"),
                epoch(rest),
                req(rest, "--state"),
            )
            .expect("engine");
            let note_id = req(rest, "--note");
            let outcome = engine.push_note(&library, &note_id).expect("push note");
            emit(json!({"note": note_id, "outcome": outcome}));
        }
        "push-canvas" => {
            let library = Library::open(req(rest, "--library")).expect("open library");
            let mut engine = SyncEngine::with_state(
                client(rest),
                load_wsk(rest),
                req(rest, "--workspace-id"),
                epoch(rest),
                req(rest, "--state"),
            )
            .expect("engine");
            let canvas_id = req(rest, "--canvas");
            let outcome = engine
                .push_canvas(&library, &canvas_id)
                .expect("push canvas");
            emit(json!({"canvas": canvas_id, "outcome": outcome}));
        }
        "trash" => {
            let mut library = Library::open(req(rest, "--library")).expect("open library");
            let note_id = req(rest, "--note");
            library.set_trashed(&note_id, true).expect("trash");
            let mut engine = SyncEngine::with_state(
                client(rest),
                load_wsk(rest),
                req(rest, "--workspace-id"),
                epoch(rest),
                req(rest, "--state"),
            )
            .expect("engine");
            let outcome = engine.push_delete(&library, &note_id).expect("push delete");
            emit(json!({"note": note_id, "outcome": outcome}));
        }
        "restore" => {
            let mut library = Library::open(req(rest, "--library")).expect("open library");
            let note_id = req(rest, "--note");
            library.set_trashed(&note_id, false).expect("restore");
            let mut engine = SyncEngine::with_state(
                client(rest),
                load_wsk(rest),
                req(rest, "--workspace-id"),
                epoch(rest),
                req(rest, "--state"),
            )
            .expect("engine");
            let outcome = engine
                .push_restore(&library, &note_id)
                .expect("push restore");
            emit(json!({"note": note_id, "outcome": outcome}));
        }
        "status" => {
            let library = Library::open(req(rest, "--library")).expect("open library");
            let note = library.note(&req(rest, "--note")).expect("note");
            emit(json!({"id": note.id, "title": note.title, "trashed": note.trashed}));
        }
        "revoke" => {
            let record = client(rest)
                .revoke_device(&req(rest, "--target"), &req(rest, "--reason"))
                .expect("revoke");
            emit(json!({"revoked": record.device_id, "display_name": record.display_name}));
        }
        "show" => {
            let library = Library::open(req(rest, "--library")).expect("open library");
            let notes: Vec<_> = library
                .list_notes()
                .expect("notes")
                .into_iter()
                .map(|note| {
                    json!({
                        "id": note.id,
                        "title": note.title,
                        "body": note.document.plain_text(),
                        "trashed": note.trashed,
                    })
                })
                .collect();
            emit(json!({"notes": notes}));
        }
        "resolve" => {
            let mut library = Library::open(req(rest, "--library")).expect("open library");
            let workspace_id = req(rest, "--workspace-id");
            let mut engine = SyncEngine::with_state(
                client(rest),
                load_wsk(rest),
                workspace_id.clone(),
                epoch(rest),
                req(rest, "--state"),
            )
            .expect("engine");
            let conflict_id = req(rest, "--conflict-id");
            let batch = engine
                .client()
                .pull(&workspace_id, 0, 100)
                .expect("list conflicts");
            let record: ConflictRecord = batch
                .conflicts
                .into_iter()
                .find(|entry| entry.conflict_id == conflict_id)
                .expect("conflict not found");
            let result = match req(rest, "--mode").as_str() {
                "keep-local" => json!({
                    "mode": "keep-local",
                    "outcome": engine.resolve_keep_local(&library, &record).expect("resolve"),
                }),
                "keep-remote" => {
                    engine
                        .resolve_keep_remote(&mut library, &record)
                        .expect("resolve");
                    json!({"mode": "keep-remote", "applied": record.object_id})
                }
                "save-as-new" => json!({
                    "mode": "save-as-new",
                    "new_object": engine
                        .resolve_save_as_new(&mut library, &record)
                        .expect("resolve"),
                }),
                other => panic!("unknown resolve mode {other}"),
            };
            emit(result);
        }
        other => panic!("unknown command {other}"),
    }
}

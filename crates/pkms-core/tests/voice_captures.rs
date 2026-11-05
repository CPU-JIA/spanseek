use pkms_core::{
    CreateVoiceCapture, DOCUMENT_SCHEMA, InboxState, Library, NotesFilter, NotesPageRequest,
    SaveVoiceCaptureNote, UpdateAsrSettings, UpdateVoiceCapture, VoiceCaptureStatus,
    VoiceCaptureTrashRequest, VoiceSourceKind, VoiceTranscriptSource,
};
use rusqlite::Connection;
use tempfile::tempdir;
use uuid::Uuid;

const AUDIO: &[u8] = b"\x1a\x45\xdf\xa3voice-capture-test-payload";
const AUDIO_2: &[u8] = b"OggSsecond-voice-capture-payload";

#[test]
fn voice_browse_can_reach_records_beyond_the_legacy_list_limit() {
    let directory = tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let first = create(&mut library);
    for _ in 0..200 {
        create(&mut library);
    }
    let query = pkms_core::VoiceBrowseRequest {
        limit: 10,
        page: 21,
        trashed: false,
        query: None,
    };
    let page = library.voice_browse(query.clone()).unwrap();
    assert_eq!((page.total, page.page, page.items.len()), (201, 21, 1));
    assert_eq!(page.items[0].id, first.id);
    assert!(page.next_cursor.is_none());
    assert_eq!(
        library
            .voice_browse(pkms_core::VoiceBrowseRequest {
                page: u32::MAX,
                ..query.clone()
            })
            .unwrap()
            .page,
        21
    );
    assert!(
        library
            .voice_browse(pkms_core::VoiceBrowseRequest {
                page: 0,
                ..query.clone()
            })
            .is_err()
    );
    let empty = library
        .voice_browse(pkms_core::VoiceBrowseRequest {
            query: Some("不存在的录音".into()),
            ..query
        })
        .unwrap();
    assert_eq!((empty.total, empty.page, empty.items.len()), (0, 1, 0));
}

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn create(library: &mut Library) -> pkms_core::VoiceCapture {
    create_bytes(library, AUDIO)
}

fn create_bytes(library: &mut Library, bytes: &[u8]) -> pkms_core::VoiceCapture {
    library
        .create_voice_capture(
            &id(),
            CreateVoiceCapture {
                title: "   ".into(),
                duration_ms: 4_200,
                source_kind: VoiceSourceKind::Local,
                device_label: Some(" 内置麦克风 ".into()),
                language: Some("zh".into()),
            },
            bytes,
        )
        .unwrap()
}

fn inbox_ids(library: &Library) -> Vec<String> {
    library
        .notes_page(NotesPageRequest {
            limit: 20,
            query: None,
            cursor: None,
            trashed: false,
            filter: Some(NotesFilter {
                inbox_state: Some(InboxState::Pending),
                ..Default::default()
            }),
            sort: None,
        })
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.id)
        .collect()
}

#[test]
fn voice_capture_round_trips_audio_and_manual_transcript() {
    let directory = tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let capture = create(&mut library);
    assert_eq!(capture.status, VoiceCaptureStatus::Pending);
    assert_eq!(capture.source_kind, VoiceSourceKind::Local);
    assert_eq!(capture.audio_media_type, "audio/webm");
    assert_eq!(capture.audio_bytes, AUDIO.len() as u32);
    assert_eq!(capture.title, "语音速记");
    assert_eq!(capture.device_label.as_deref(), Some("内置麦克风"));
    assert_eq!(
        library.voice_capture_audio(&capture.id).unwrap(),
        AUDIO.to_vec()
    );
    assert_eq!(library.voice_captures(false).unwrap().items.len(), 1);

    let updated = library
        .update_voice_capture(UpdateVoiceCapture {
            capture_id: capture.id.clone(),
            base_row_version: capture.row_version,
            title: "产品灵感".into(),
            transcript: "第一行\n第二行".into(),
        })
        .unwrap();
    assert_eq!(updated.status, VoiceCaptureStatus::Ready);
    assert_eq!(updated.transcript_source, VoiceTranscriptSource::Manual);
    assert_eq!(updated.title, "产品灵感");
    assert!(updated.row_version > capture.row_version);

    let cleared = library
        .update_voice_capture(UpdateVoiceCapture {
            capture_id: capture.id.clone(),
            base_row_version: updated.row_version,
            title: "产品灵感".into(),
            transcript: "   ".into(),
        })
        .unwrap();
    assert_eq!(cleared.status, VoiceCaptureStatus::Pending);
    assert_eq!(cleared.transcript_source, VoiceTranscriptSource::None);
    assert_eq!(
        library.voice_capture_audio(&capture.id).unwrap(),
        AUDIO.to_vec(),
        "editing text must never rewrite the original audio"
    );
}

#[test]
fn voice_capture_promotes_to_an_idea_note_in_the_inbox() {
    let directory = tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let capture = create(&mut library);
    let ready = library
        .update_voice_capture(UpdateVoiceCapture {
            capture_id: capture.id.clone(),
            base_row_version: capture.row_version,
            title: "语音速记".into(),
            transcript: "把语音输入做成完整链路\n第一阶段先落本机".into(),
        })
        .unwrap();
    let promoted = library
        .save_voice_capture_note(
            &id(),
            SaveVoiceCaptureNote {
                capture_id: capture.id.clone(),
                base_row_version: ready.row_version,
                collection_id: None,
                base_collection_row_version: None,
                tag_names: vec!["语音".into(), "复盘".into()],
            },
        )
        .unwrap();
    assert_eq!(promoted.note_id.as_deref(), Some(capture.id.as_str()));
    let note = library.note(&capture.id).unwrap();
    assert_eq!(note.schema_version, DOCUMENT_SCHEMA);
    assert_eq!(
        note.document.content[0].kind,
        pkms_core::NodeType::Frontmatter
    );
    let yaml = note.document.content[0].attrs.source.as_deref().unwrap();
    assert!(yaml.contains("type: idea"));
    assert!(yaml.contains(&format!("voice_capture: {}", capture.id)));
    assert!(
        note.document
            .plain_text()
            .contains("把语音输入做成完整链路")
    );
    assert!(
        inbox_ids(&library).contains(&capture.id),
        "a promoted capture must land in the pending inbox"
    );
    let metadata = library.object_metadata(&capture.id).unwrap();
    let names = metadata
        .tags
        .iter()
        .map(|tag| tag.display_name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"语音"));
    assert!(names.contains(&"复盘"));

    // A retry must not create a second note or bump the capture twice.
    let replay = library
        .save_voice_capture_note(
            &id(),
            SaveVoiceCaptureNote {
                capture_id: capture.id.clone(),
                base_row_version: promoted.row_version,
                collection_id: None,
                base_collection_row_version: None,
                tag_names: vec![],
            },
        )
        .unwrap();
    assert_eq!(replay.note_id.as_deref(), Some(capture.id.as_str()));
    assert_eq!(inbox_ids(&library).len(), 1);
}

#[test]
fn transcription_states_preserve_audio_and_recover_after_restart() {
    let directory = tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let capture = create(&mut library);
    let running = library
        .begin_voice_transcription(&capture.id, Some(capture.row_version))
        .unwrap();
    assert_eq!(running.status, VoiceCaptureStatus::Transcribing);
    let done = library
        .finish_voice_transcription(
            &capture.id,
            running.row_version,
            "模型转写文本",
            &[pkms_core::VoiceSegment {
                start_ms: 0,
                end_ms: 900,
                text: "模型转写文本".into(),
                speaker: None,
            }],
        )
        .unwrap();
    assert_eq!(done.status, VoiceCaptureStatus::Ready);
    assert_eq!(done.transcript_source, VoiceTranscriptSource::Asr);
    assert_eq!(done.transcript, "模型转写文本");
    assert_eq!(done.segments.len(), 1);
    assert_eq!(done.segments[0].end_ms, 900);
    assert_eq!(
        library.voice_capture_audio(&capture.id).unwrap(),
        AUDIO.to_vec()
    );

    let empty = library
        .begin_voice_transcription(&capture.id, Some(done.row_version))
        .unwrap();
    let failed = library
        .finish_voice_transcription(&capture.id, empty.row_version, "   ", &[])
        .unwrap();
    assert_eq!(failed.status, VoiceCaptureStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("asr_empty_transcript"));
    assert_eq!(
        library.voice_capture_audio(&capture.id).unwrap(),
        AUDIO.to_vec()
    );

    let second = create_bytes(&mut library, AUDIO_2);
    library
        .begin_voice_transcription(&second.id, Some(second.row_version))
        .unwrap();
    drop(library);
    // A new process cannot have an in-flight request for an exclusively locked
    // workspace, so the stale row becomes retryable rather than eternally busy.
    let library = Library::open(directory.path()).unwrap();
    let recovered = library.voice_capture(&second.id).unwrap();
    assert_eq!(recovered.status, VoiceCaptureStatus::Failed);
    assert_eq!(recovered.error_code.as_deref(), Some("asr_interrupted"));
    assert_eq!(
        library.voice_capture_audio(&second.id).unwrap(),
        AUDIO_2.to_vec()
    );
}

#[test]
fn trash_keeps_audio_and_lists_are_separated() {
    let directory = tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let capture = create(&mut library);
    let trashed = library
        .trash_voice_capture(VoiceCaptureTrashRequest {
            capture_id: capture.id.clone(),
            base_row_version: capture.row_version,
            trashed: true,
        })
        .unwrap();
    assert!(trashed.trashed);
    assert!(library.voice_captures(false).unwrap().items.is_empty());
    assert_eq!(library.voice_captures(true).unwrap().items.len(), 1);
    assert!(matches!(
        library.voice_capture_audio(&capture.id),
        Err(pkms_core::Error::Trashed)
    ));
    let restored = library
        .trash_voice_capture(VoiceCaptureTrashRequest {
            capture_id: capture.id.clone(),
            base_row_version: trashed.row_version,
            trashed: false,
        })
        .unwrap();
    assert!(!restored.trashed);
    assert_eq!(
        library.voice_capture_audio(&capture.id).unwrap(),
        AUDIO.to_vec()
    );
}

#[test]
fn asr_settings_are_workspace_scoped_and_versioned() {
    let directory = tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let initial = library.asr_settings().unwrap();
    assert!(initial.endpoint.is_empty());
    assert_eq!(initial.row_version, 1);

    let saved = library
        .update_asr_settings(UpdateAsrSettings {
            base_row_version: initial.row_version,
            endpoint: " https://api.example.test/v1 ".into(),
            model: " whisper-1 ".into(),
            language: "zh".into(),
        })
        .unwrap();
    assert_eq!(saved.endpoint, "https://api.example.test/v1");
    assert_eq!(saved.model, "whisper-1");
    assert_eq!(saved.row_version, 2);

    assert!(matches!(
        library.update_asr_settings(UpdateAsrSettings {
            base_row_version: initial.row_version,
            endpoint: "https://api.example.test/v1".into(),
            model: "whisper-1".into(),
            language: "zh".into(),
        }),
        Err(pkms_core::Error::Conflict { .. })
    ));
    let invalid = library.update_asr_settings(UpdateAsrSettings {
        base_row_version: saved.row_version,
        endpoint: "http://api.example.test/v1".into(),
        model: "whisper-1".into(),
        language: "zh".into(),
    });
    assert!(matches!(invalid, Err(pkms_core::Error::InvalidInput(_))));

    let cleared = library
        .update_asr_settings(UpdateAsrSettings {
            base_row_version: saved.row_version,
            endpoint: String::new(),
            model: String::new(),
            language: String::new(),
        })
        .unwrap();
    assert!(cleared.endpoint.is_empty());
}

#[test]
fn portable_backup_counts_and_carries_voice_audio() {
    let directory = tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let capture = create(&mut library);
    let backup = directory.path().join("voice-backup.zip");
    library.create_backup(&backup, &mut |_| Ok(())).unwrap();
    let inspection = pkms_core::inspect_backup(&backup).unwrap();
    assert_eq!(inspection.manifest.counts.voice_captures, 1);
    assert!(
        inspection
            .manifest
            .blobs
            .iter()
            .any(|blob| blob.sha256 == capture.audio_blob_hash),
        "the raw audio blob must be part of the portable backup"
    );
    let records: i64 = Connection::open(directory.path().join("library.sqlite3"))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM voice_captures WHERE status='pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(records, 1);
}

#[test]
fn fresh_workspace_publishes_current_schema() {
    let directory = tempdir().unwrap();
    let library = Library::open(directory.path()).unwrap();
    assert_eq!(
        library.info().unwrap().database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
}

#[test]
fn voice_sync_task_records_status_and_increments_row_version() {
    let directory = tempfile::tempdir().unwrap();
    let mut library = pkms_core::Library::open(directory.path()).unwrap();
    let first = library
        .record_voice_sync_task("a".repeat(64).as_str(), "syncing", None, None, None)
        .unwrap();
    assert_eq!(first.status, "syncing");
    assert_eq!(first.row_version, 1);
    let second = library
        .record_voice_sync_task(
            "a".repeat(64).as_str(),
            "failed",
            None,
            None,
            Some("voice_sync_failed"),
        )
        .unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.status, "failed");
    assert_eq!(second.row_version, 2);
    let tasks = library.voice_sync_tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].error_code.as_deref(), Some("voice_sync_failed"));
}

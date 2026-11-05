//! Voice memos: audio-first capture, optional transcription, explicit note promotion.
//!
//! The raw audio blob is published before the row is committed and is never
//! rewritten by transcription or editing. Every mutating call uses CAS.
use crate::{
    AsrSettings, AssignTag, CreateVoiceCapture, Error, Library, MetadataTarget, Result,
    SaveVoiceCaptureNote, TagSelector, UpdateAsrSettings, UpdateVoiceCapture, VOICE_MAX_BYTES,
    VOICE_MAX_IMPORT_DURATION_MS, VOICE_MAX_LIST_ITEMS, VOICE_MAX_LOCAL_DURATION_MS,
    VOICE_MAX_TRANSCRIPT_CHARS, VoiceCapture, VoiceCapturePage, VoiceCaptureStatus,
    VoiceCaptureTrashRequest, VoiceSegment, VoiceSourceKind, VoiceTranscriptSource,
    document::DOCUMENT_SCHEMA, document::DocumentNode, document::NodeAttributes,
    document::NodeType, document::NoteDocument,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use url::Url;
use uuid::Uuid;

const CREATE_KIND: &str = "voice_capture_create";

#[derive(Debug, Serialize, Deserialize)]
struct CreateResult {
    capture: VoiceCapture,
}

impl Library {
    pub fn voice_browse(
        &self,
        request: crate::VoiceBrowseRequest,
    ) -> Result<crate::VoiceBrowsePage> {
        crate::collections::validate_limit(request.limit)?;
        if request.page == 0 {
            return Err(Error::InvalidInput("invalid voice page"));
        }
        let query = request
            .query
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if query.chars().count() > 512 || query.chars().any(char::is_control) {
            return Err(Error::InvalidInput("invalid voice query"));
        }
        let snapshot = self.connection.unchecked_transaction()?;
        let predicate =
            "trashed=?1 AND (?2='' OR instr(lower(title),?2)>0 OR instr(lower(transcript),?2)>0)";
        let total: u32 = snapshot.query_row(
            &format!("SELECT count(*) FROM voice_captures WHERE {predicate}"),
            params![request.trashed, query],
            |row| row.get(0),
        )?;
        let page = request.page.min(total.div_ceil(request.limit).max(1));
        let offset = i64::from(page - 1) * i64::from(request.limit);
        let mut statement = snapshot.prepare(&format!(
            "SELECT id,title,status,audio_blob_hash,audio_media_type,audio_bytes,duration_ms,
             transcript,segments_json,transcript_source,source_kind,device_label,language,error_code,
             note_id,created_at,updated_at,row_version,trashed
             FROM voice_captures WHERE {predicate} ORDER BY created_at DESC,id DESC LIMIT ?3 OFFSET ?4"))?;
        let items = statement
            .query_map(
                params![request.trashed, query, request.limit, offset],
                row_capture,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(crate::VoiceBrowsePage {
            items,
            total,
            page,
            page_size: request.limit,
            next_cursor: (page < total.div_ceil(request.limit)).then(|| (page + 1).to_string()),
        })
    }

    pub fn create_voice_capture(
        &mut self,
        command_id: &str,
        request: CreateVoiceCapture,
        bytes: &[u8],
    ) -> Result<VoiceCapture> {
        crate::document::valid_uuid(command_id)?;
        let request = normalize_create(request)?;
        let media_type = detect_audio(bytes).ok_or(Error::InvalidInput("voice_audio_invalid"))?;
        if bytes.is_empty() || bytes.len() > VOICE_MAX_BYTES {
            return Err(Error::InvalidInput("voice_audio_too_large"));
        }
        let digest = format!("{:x}", Sha256::digest(bytes));
        let payload_hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                1,
                CREATE_KIND,
                &request,
                &digest,
                bytes.len() as u64,
            ))?)
        );
        if let Some(existing) = replay_capture(&self.connection, command_id, &payload_hash)? {
            // A receipt is only trusted while its audio is still readable.
            self.read_voice_audio(&existing.audio_blob_hash, existing.audio_bytes)?;
            return Ok(existing);
        }
        self.publish_blob(
            &crate::BlobInfo {
                hash: digest.clone(),
                size: bytes.len() as u32,
                media_type: media_type.into(),
                width: 0,
                height: 0,
            },
            bytes,
        )?;
        let now = Utc::now().to_rfc3339();
        let id = Uuid::new_v4().to_string();
        let title = if request.title.is_empty() {
            "语音速记".to_owned()
        } else {
            request.title
        };
        let capture = VoiceCapture {
            id: id.clone(),
            title,
            status: VoiceCaptureStatus::Pending,
            audio_blob_hash: digest.clone(),
            audio_media_type: media_type.into(),
            audio_bytes: bytes.len() as u32,
            duration_ms: request.duration_ms,
            transcript: String::new(),
            segments: vec![],
            transcript_source: VoiceTranscriptSource::None,
            source_kind: request.source_kind,
            device_label: request.device_label,
            language: request.language,
            error_code: None,
            note_id: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            row_version: 1,
            trashed: false,
        };
        let result_json = serde_json::to_string(&CreateResult {
            capture: capture.clone(),
        })?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = replay_capture(&tx, command_id, &payload_hash)? {
            return Ok(existing);
        }
        tx.execute(
            "INSERT INTO blobs(hash,size,media_type,width,height,created_at) VALUES(?1,?2,?3,NULL,NULL,?4) ON CONFLICT(hash) DO NOTHING",
            params![capture.audio_blob_hash, capture.audio_bytes, capture.audio_media_type, now],
        )?;
        tx.execute(
            "INSERT INTO voice_captures(
                id,title,status,audio_blob_hash,audio_media_type,audio_bytes,duration_ms,
                transcript,segments_json,transcript_source,source_kind,device_label,language,error_code,
                note_id,created_at,updated_at,row_version,trashed
             ) VALUES(?1,?2,'pending',?3,?4,?5,?6,'','[]','none',?7,?8,?9,NULL,NULL,?10,?11,1,0)",
            params![
                capture.id,
                capture.title,
                capture.audio_blob_hash,
                capture.audio_media_type,
                capture.audio_bytes,
                capture.duration_ms,
                source_wire(capture.source_kind),
                capture.device_label,
                capture.language,
                capture.created_at,
                capture.updated_at,
            ],
        )?;
        let seq: i64 = tx.query_row(
            "SELECT next_change_seq FROM workspace_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let next = seq
            .checked_add(1)
            .ok_or(Error::InvalidInput("change sequence overflow"))?;
        tx.execute(
            "UPDATE workspace_meta SET next_change_seq=?1 WHERE singleton=1",
            [next],
        )?;
        tx.execute(
            "INSERT INTO command_receipts(command_id,payload_hash,command_kind,object_id,revision_id,change_seq,committed_at,result_json)
             VALUES(?1,?2,?3,?4,NULL,?5,?6,?7)",
            params![command_id, payload_hash, CREATE_KIND, capture.id, seq, capture.created_at, result_json],
        )?;
        tx.commit()?;
        Ok(capture)
    }

    pub fn voice_captures(&self, trashed: bool) -> Result<VoiceCapturePage> {
        let mut statement = self.connection.prepare(
            "SELECT id,title,status,audio_blob_hash,audio_media_type,audio_bytes,duration_ms,
                    transcript,segments_json,transcript_source,source_kind,device_label,language,error_code,
                    note_id,created_at,updated_at,row_version,trashed
             FROM voice_captures WHERE trashed=?1
             ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let items = statement
            .query_map(
                params![i64::from(trashed), VOICE_MAX_LIST_ITEMS],
                row_capture,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(VoiceCapturePage { items })
    }

    pub fn voice_capture(&self, capture_id: &str) -> Result<VoiceCapture> {
        crate::document::valid_uuid(capture_id)?;
        self.connection
            .query_row(
                "SELECT id,title,status,audio_blob_hash,audio_media_type,audio_bytes,duration_ms,
                        transcript,segments_json,transcript_source,source_kind,device_label,language,error_code,
                        note_id,created_at,updated_at,row_version,trashed
                 FROM voice_captures WHERE id=?1",
                [capture_id],
                row_capture,
            )
            .optional()?
            .ok_or(Error::NotFound)
    }

    pub fn update_voice_capture(&mut self, request: UpdateVoiceCapture) -> Result<VoiceCapture> {
        crate::document::valid_uuid(&request.capture_id)?;
        let title = request.title.trim().to_owned();
        if title.is_empty() || title.chars().count() > 120 {
            return Err(Error::InvalidInput("voice_title_invalid"));
        }
        if request.transcript.chars().count() > VOICE_MAX_TRANSCRIPT_CHARS {
            return Err(Error::InvalidInput("voice_transcript_too_long"));
        }
        let existing = self.voice_capture(&request.capture_id)?;
        if existing.trashed {
            return Err(Error::Trashed);
        }
        let has_text = !request.transcript.trim().is_empty();
        let (status, source, error): (
            VoiceCaptureStatus,
            VoiceTranscriptSource,
            Option<&'static str>,
        ) = if has_text {
            (
                VoiceCaptureStatus::Ready,
                VoiceTranscriptSource::Manual,
                None,
            )
        } else {
            (
                VoiceCaptureStatus::Pending,
                VoiceTranscriptSource::None,
                None,
            )
        };
        let now = Utc::now().to_rfc3339();
        let changed = self.connection.execute(
            "UPDATE voice_captures SET title=?1,transcript=?2,segments_json='[]',transcript_source=?3,status=?4,
                    error_code=?5,updated_at=?6,row_version=row_version+1
             WHERE id=?7 AND row_version=?8 AND trashed=0",
            params![
                title,
                request.transcript,
                transcript_wire(source),
                status_wire(status),
                error,
                now,
                request.capture_id,
                request.base_row_version,
            ],
        )?;
        if changed == 0 {
            return conflict_after_miss(self, &request.capture_id, request.base_row_version);
        }
        self.voice_capture(&request.capture_id)
    }

    pub fn trash_voice_capture(
        &mut self,
        request: VoiceCaptureTrashRequest,
    ) -> Result<VoiceCapture> {
        crate::document::valid_uuid(&request.capture_id)?;
        let now = Utc::now().to_rfc3339();
        let changed = self.connection.execute(
            "UPDATE voice_captures SET trashed=?1,updated_at=?2,row_version=row_version+1
             WHERE id=?3 AND row_version=?4",
            params![
                i64::from(request.trashed),
                now,
                request.capture_id,
                request.base_row_version
            ],
        )?;
        if changed == 0 {
            return conflict_after_miss(self, &request.capture_id, request.base_row_version);
        }
        self.voice_capture(&request.capture_id)
    }

    /// Promote the capture into a normal inbox note. The note reuses the capture
    /// UUID, making a retry after a crash deterministic instead of duplicating.
    pub fn save_voice_capture_note(
        &mut self,
        command_id: &str,
        request: SaveVoiceCaptureNote,
    ) -> Result<VoiceCapture> {
        crate::document::valid_uuid(command_id)?;
        let capture = self.voice_capture(&request.capture_id)?;
        if capture.trashed {
            return Err(Error::Trashed);
        }
        let mut tags = Vec::new();
        for name in &request.tag_names {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
                return Err(Error::InvalidInput("voice_tag_invalid"));
            }
            if !tags.iter().any(|existing| existing == name) {
                tags.push(name.to_owned());
            }
        }
        if tags.len() > 12 {
            return Err(Error::InvalidInput("voice_tag_limit"));
        }
        match (
            request.collection_id.as_deref(),
            request.base_collection_row_version,
        ) {
            (None, None) | (Some(_), Some(_)) => {}
            _ => {
                return Err(Error::InvalidInput(
                    "collection ID and base version must be paired",
                ));
            }
        }
        if let Some(note_id) = capture.note_id.as_deref() {
            // Idempotent: the capture already points at its promoted note, but a
            // retry may still need to attach tags that failed previously.
            if self.contains_object(note_id)? {
                self.assign_voice_tags(note_id, &tags)?;
                return Ok(capture);
            }
        }
        if capture.row_version != request.base_row_version {
            return Err(Error::Conflict {
                expected: request.base_row_version,
                actual: capture.row_version,
            });
        }
        if capture.transcript.trim().is_empty() {
            return Err(Error::InvalidInput("voice_transcript_empty"));
        }
        let title = note_title(&capture);
        let document = note_document(&capture);
        let note = if self.contains_object(&capture.id)? {
            let existing = crate::notes::read_note(&self.connection, &capture.id)?;
            if existing.trashed {
                return Err(Error::Trashed);
            }
            existing
        } else {
            self.create_document_with_id(
                command_id,
                &capture.id,
                crate::CreateNote {
                    title,
                    document_schema: DOCUMENT_SCHEMA,
                    document,
                    collection_id: request.collection_id.clone(),
                    base_collection_row_version: request.base_collection_row_version,
                },
            )?
            .note
        };
        let now = Utc::now().to_rfc3339();
        let changed = self.connection.execute(
            "UPDATE voice_captures SET note_id=?1,updated_at=?2,row_version=row_version+1
             WHERE id=?3 AND row_version=?4 AND note_id IS NULL",
            params![note.id, now, capture.id, request.base_row_version],
        )?;
        if changed == 0 {
            return conflict_after_miss(self, &capture.id, request.base_row_version);
        }
        self.assign_voice_tags(&capture.id, &tags)?;
        self.voice_capture(&capture.id)
    }

    fn assign_voice_tags(&mut self, note_id: &str, tags: &[String]) -> Result<()> {
        for name in tags {
            let metadata = self.object_metadata(note_id)?;
            if metadata
                .tags
                .iter()
                .any(|tag| tag.display_name.eq_ignore_ascii_case(name))
            {
                continue;
            }
            self.assign_tag(
                &Uuid::new_v4().to_string(),
                AssignTag {
                    targets: vec![MetadataTarget {
                        object_id: note_id.to_owned(),
                        base_row_version: metadata.row_version,
                    }],
                    tag: TagSelector::Name { name: name.clone() },
                },
            )?;
        }
        Ok(())
    }

    pub fn voice_capture_audio(&self, capture_id: &str) -> Result<Vec<u8>> {
        let capture = self.voice_capture(capture_id)?;
        if capture.trashed {
            return Err(Error::Trashed);
        }
        self.read_voice_audio(&capture.audio_blob_hash, capture.audio_bytes)
    }

    /// Marks the row before a network call. If the process dies mid-call, the
    /// next workspace open turns this state into a retryable failure.
    pub fn begin_voice_transcription(
        &mut self,
        capture_id: &str,
        base_row_version: Option<u32>,
    ) -> Result<VoiceCapture> {
        let capture = self.voice_capture(capture_id)?;
        if capture.trashed {
            return Err(Error::Trashed);
        }
        if capture.status == VoiceCaptureStatus::Transcribing {
            return Err(Error::InvalidInput("asr_already_running"));
        }
        let expected = base_row_version.unwrap_or(capture.row_version);
        if expected != capture.row_version {
            return Err(Error::Conflict {
                expected,
                actual: capture.row_version,
            });
        }
        let now = Utc::now().to_rfc3339();
        let changed = self.connection.execute(
            "UPDATE voice_captures SET status='transcribing',error_code=NULL,updated_at=?1,
                    row_version=row_version+1
             WHERE id=?2 AND row_version=?3 AND trashed=0",
            params![now, capture_id, expected],
        )?;
        if changed == 0 {
            return conflict_after_miss(self, capture_id, expected);
        }
        self.voice_capture(capture_id)
    }

    pub fn finish_voice_transcription(
        &mut self,
        capture_id: &str,
        expected_row_version: u32,
        transcript: &str,
        segments: &[VoiceSegment],
    ) -> Result<VoiceCapture> {
        if transcript.trim().is_empty() {
            return self.fail_voice_transcription(
                capture_id,
                expected_row_version,
                "asr_empty_transcript",
            );
        }
        if transcript.chars().count() > VOICE_MAX_TRANSCRIPT_CHARS {
            return self.fail_voice_transcription(
                capture_id,
                expected_row_version,
                "asr_transcript_too_long",
            );
        }
        if segments.iter().any(|segment| {
            segment.start_ms > segment.end_ms || segment.text.chars().count() > 20_000
        }) {
            return self.fail_voice_transcription(
                capture_id,
                expected_row_version,
                "asr_invalid_response",
            );
        }
        let segments_json = serde_json::to_string(segments)?;
        let now = Utc::now().to_rfc3339();
        let changed = self.connection.execute(
            "UPDATE voice_captures SET transcript=?1,segments_json=?2,transcript_source='asr',status='ready',
                    error_code=NULL,updated_at=?3,row_version=row_version+1
             WHERE id=?4 AND row_version=?5",
            params![
                transcript,
                segments_json,
                now,
                capture_id,
                expected_row_version
            ],
        )?;
        if changed == 0 {
            return conflict_after_miss(self, capture_id, expected_row_version);
        }
        self.voice_capture(capture_id)
    }

    pub fn fail_voice_transcription(
        &mut self,
        capture_id: &str,
        expected_row_version: u32,
        error_code: &str,
    ) -> Result<VoiceCapture> {
        if error_code.is_empty() || error_code.len() > 64 {
            return Err(Error::InvalidInput("asr_error_code_invalid"));
        }
        let now = Utc::now().to_rfc3339();
        let changed = self.connection.execute(
            "UPDATE voice_captures SET status='failed',error_code=?1,updated_at=?2,
                    row_version=row_version+1
             WHERE id=?3 AND row_version=?4",
            params![error_code, now, capture_id, expected_row_version],
        )?;
        if changed == 0 {
            return conflict_after_miss(self, capture_id, expected_row_version);
        }
        self.voice_capture(capture_id)
    }

    pub fn asr_settings(&self) -> Result<AsrSettings> {
        self.connection
            .query_row(
                "SELECT endpoint,model,language,row_version,updated_at FROM asr_settings WHERE singleton=1",
                [],
                |row| {
                    Ok(AsrSettings {
                        endpoint: row.get(0)?,
                        model: row.get(1)?,
                        language: row.get(2)?,
                        row_version: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub fn update_asr_settings(&mut self, request: UpdateAsrSettings) -> Result<AsrSettings> {
        let endpoint = request.endpoint.trim().to_owned();
        let model = request.model.trim().to_owned();
        let language = request.language.trim().to_owned();
        validate_settings(&endpoint, &model, &language)?;
        let now = Utc::now().to_rfc3339();
        let changed = self.connection.execute(
            "UPDATE asr_settings SET endpoint=?1,model=?2,language=?3,updated_at=?4,
                    row_version=row_version+1
             WHERE singleton=1 AND row_version=?5",
            params![endpoint, model, language, now, request.base_row_version],
        )?;
        if changed == 0 {
            let actual: Option<u32> = self
                .connection
                .query_row(
                    "SELECT row_version FROM asr_settings WHERE singleton=1",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            return Err(match actual {
                Some(actual) => Error::Conflict {
                    expected: request.base_row_version,
                    actual,
                },
                None => Error::IntegrityFailure,
            });
        }
        self.asr_settings()
    }

    /// Any `transcribing` row is stale after a process restart; no request can
    /// still be in flight because the workspace lock is exclusive.
    pub(crate) fn recover_interrupted_voice_transcriptions(&mut self) -> Result<()> {
        self.connection.execute(
            "UPDATE voice_captures SET status='failed',error_code='asr_interrupted',
                    updated_at=?1,row_version=row_version+1
             WHERE status='transcribing'",
            [Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn read_voice_audio(&self, hash: &str, expected_bytes: u32) -> Result<Vec<u8>> {
        let size: i64 = self
            .connection
            .query_row("SELECT size FROM blobs WHERE hash=?1", [hash], |row| {
                row.get(0)
            })
            .optional()?
            .ok_or(Error::NotFound)?;
        if size <= 0 || size as usize != expected_bytes as usize || size as usize > VOICE_MAX_BYTES
        {
            return Err(Error::IntegrityFailure);
        }
        let path = self.blob_path(hash, false)?;
        let file = std::fs::File::open(path)?;
        let mut bytes = Vec::with_capacity(size as usize);
        file.take(VOICE_MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() != size as usize || format!("{:x}", Sha256::digest(&bytes)) != hash {
            return Err(Error::IntegrityFailure);
        }
        Ok(bytes)
    }
}

fn normalize_create(mut request: CreateVoiceCapture) -> Result<CreateVoiceCapture> {
    request.title = request.title.trim().to_owned();
    if request.title.chars().count() > 120
        || request
            .title
            .chars()
            .any(|character| character.is_control())
    {
        return Err(Error::InvalidInput("voice_title_invalid"));
    }
    let max_duration = match request.source_kind {
        VoiceSourceKind::Local => VOICE_MAX_LOCAL_DURATION_MS,
        VoiceSourceKind::Import => VOICE_MAX_IMPORT_DURATION_MS,
    };
    if request.duration_ms > max_duration {
        return Err(Error::InvalidInput("voice_duration_too_long"));
    }
    if let Some(label) = request.device_label.as_mut() {
        *label = label.trim().to_owned();
        if label.chars().count() > 80 || label.chars().any(char::is_control) {
            return Err(Error::InvalidInput("voice_device_label_invalid"));
        }
        if label.is_empty() {
            request.device_label = None;
        }
    }
    if let Some(language) = request.language.as_mut() {
        *language = language.trim().to_owned();
        if language.len() > 32
            || !language
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(Error::InvalidInput("voice_language_invalid"));
        }
        if language.is_empty() {
            request.language = None;
        }
    }
    Ok(request)
}

fn validate_settings(endpoint: &str, model: &str, language: &str) -> Result<()> {
    if endpoint.is_empty() && model.is_empty() {
        if language.is_empty() {
            return Ok(());
        }
        return Err(Error::InvalidInput("asr_settings_incomplete"));
    }
    if endpoint.is_empty() || model.is_empty() {
        return Err(Error::InvalidInput("asr_settings_incomplete"));
    }
    if endpoint.len() > 2048
        || endpoint.chars().any(char::is_control)
        || model.len() > 200
        || model.chars().any(char::is_control)
        || language.len() > 32
        || !language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(Error::InvalidInput("asr_settings_invalid"));
    }
    let url = Url::parse(endpoint).map_err(|_| Error::InvalidInput("asr_endpoint_invalid"))?;
    let loopback = url
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|address| address.is_loopback());
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidInput("asr_endpoint_invalid"));
    }
    Ok(())
}

fn note_title(capture: &VoiceCapture) -> String {
    let first = capture
        .transcript
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let candidate = if capture.title == "语音速记" && !first.is_empty() {
        first
    } else {
        &capture.title
    };
    candidate.chars().take(80).collect()
}

fn note_document(capture: &VoiceCapture) -> NoteDocument {
    let mut content = vec![DocumentNode {
        kind: NodeType::Frontmatter,
        attrs: NodeAttributes {
            id: Uuid::new_v4().to_string(),
            source: Some(format!(
                "type: idea\nvoice_capture: {}\nvoice_duration_ms: {}\nvoice_recorded_at: {}\n",
                capture.id, capture.duration_ms, capture.created_at
            )),
            ..Default::default()
        },
        content: vec![],
        text: None,
        marks: vec![],
    }];
    for line in capture.transcript.lines().take(500) {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        content.push(DocumentNode {
            kind: NodeType::Paragraph,
            attrs: NodeAttributes {
                id: Uuid::new_v4().to_string(),
                ..Default::default()
            },
            content: vec![DocumentNode {
                kind: NodeType::Text,
                attrs: NodeAttributes::default(),
                content: vec![],
                text: Some(text.chars().take(20_000).collect()),
                marks: vec![],
            }],
            text: None,
            marks: vec![],
        });
    }
    content.push(DocumentNode {
        kind: NodeType::Blockquote,
        attrs: NodeAttributes {
            id: Uuid::new_v4().to_string(),
            ..Default::default()
        },
        content: vec![DocumentNode {
            kind: NodeType::Paragraph,
            attrs: NodeAttributes {
                id: Uuid::new_v4().to_string(),
                ..Default::default()
            },
            content: vec![DocumentNode {
                kind: NodeType::Text,
                attrs: NodeAttributes::default(),
                content: vec![],
                text: Some("由语音速记生成；原始音频保留在「语音速记」中。".into()),
                marks: vec![],
            }],
            text: None,
            marks: vec![],
        }],
        text: None,
        marks: vec![],
    });
    NoteDocument {
        kind: crate::DocumentType::Doc,
        content,
    }
}

fn replay_capture(
    connection: &Connection,
    command_id: &str,
    payload_hash: &str,
) -> Result<Option<VoiceCapture>> {
    let row: Option<(String, String, Option<String>)> = connection
        .query_row(
            "SELECT payload_hash,command_kind,result_json FROM command_receipts WHERE command_id=?1",
            [command_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((expected, kind, result)) = row else {
        return Ok(None);
    };
    if expected != payload_hash || kind != CREATE_KIND {
        return Err(Error::IdempotencyMismatch);
    }
    let result: CreateResult =
        serde_json::from_str(result.as_deref().ok_or(Error::IntegrityFailure)?)?;
    Ok(Some(result.capture))
}

fn row_capture(row: &Row<'_>) -> rusqlite::Result<VoiceCapture> {
    let segments_json: String = row.get(8)?;
    let segments: Vec<VoiceSegment> = serde_json::from_str(&segments_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(VoiceCapture {
        id: row.get(0)?,
        title: row.get(1)?,
        status: status_from_wire(&row.get::<_, String>(2)?),
        audio_blob_hash: row.get(3)?,
        audio_media_type: row.get(4)?,
        audio_bytes: row.get(5)?,
        duration_ms: row.get(6)?,
        transcript: row.get(7)?,
        segments,
        transcript_source: transcript_from_wire(&row.get::<_, String>(9)?),
        source_kind: source_from_wire(&row.get::<_, String>(10)?),
        device_label: row.get(11)?,
        language: row.get(12)?,
        error_code: row.get(13)?,
        note_id: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        row_version: row.get(17)?,
        trashed: row.get(18)?,
    })
}

fn status_wire(status: VoiceCaptureStatus) -> &'static str {
    match status {
        VoiceCaptureStatus::Pending => "pending",
        VoiceCaptureStatus::Transcribing => "transcribing",
        VoiceCaptureStatus::Ready => "ready",
        VoiceCaptureStatus::Failed => "failed",
    }
}
fn status_from_wire(value: &str) -> VoiceCaptureStatus {
    match value {
        "transcribing" => VoiceCaptureStatus::Transcribing,
        "ready" => VoiceCaptureStatus::Ready,
        "failed" => VoiceCaptureStatus::Failed,
        _ => VoiceCaptureStatus::Pending,
    }
}
fn transcript_wire(source: VoiceTranscriptSource) -> &'static str {
    match source {
        VoiceTranscriptSource::None => "none",
        VoiceTranscriptSource::Asr => "asr",
        VoiceTranscriptSource::Manual => "manual",
    }
}
fn transcript_from_wire(value: &str) -> VoiceTranscriptSource {
    match value {
        "asr" => VoiceTranscriptSource::Asr,
        "manual" => VoiceTranscriptSource::Manual,
        _ => VoiceTranscriptSource::None,
    }
}
fn source_wire(kind: VoiceSourceKind) -> &'static str {
    match kind {
        VoiceSourceKind::Local => "local",
        VoiceSourceKind::Import => "import",
    }
}
fn source_from_wire(value: &str) -> VoiceSourceKind {
    match value {
        "import" => VoiceSourceKind::Import,
        _ => VoiceSourceKind::Local,
    }
}
fn conflict_after_miss(library: &Library, capture_id: &str, expected: u32) -> Result<VoiceCapture> {
    match library.voice_capture(capture_id) {
        Ok(capture) if capture.trashed => Err(Error::Trashed),
        Ok(capture) => Err(Error::Conflict {
            expected,
            actual: capture.row_version,
        }),
        Err(error) => Err(error),
    }
}

fn detect_audio(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        Some("audio/webm")
    } else if bytes.starts_with(b"OggS") {
        Some("audio/ogg")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Some("audio/wav")
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        Some("audio/mp4")
    } else if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
    {
        Some("audio/mpeg")
    } else {
        None
    }
}

#[allow(dead_code)]
fn media_extension(media_type: &str) -> &'static str {
    match media_type {
        "audio/webm" => "webm",
        "audio/ogg" => "ogg",
        "audio/wav" => "wav",
        "audio/mp4" => "m4a",
        "audio/mpeg" => "mp3",
        _ => "bin",
    }
}

impl Library {
    pub fn voice_sync_tasks(&self) -> Result<Vec<crate::VoiceSyncTask>> {
        let mut statement = self.connection.prepare(
            "SELECT id,capture_id,device_id,direction,status,blob_hash,error_code,created_at,updated_at,row_version
             FROM voice_sync_tasks ORDER BY updated_at DESC, id DESC",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(crate::VoiceSyncTask {
                    id: row.get(0)?,
                    capture_id: row.get(1)?,
                    device_id: row.get(2)?,
                    direction: row.get(3)?,
                    status: row.get(4)?,
                    blob_hash: row.get(5)?,
                    error_code: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    row_version: row.get(9)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn record_voice_sync_task(
        &mut self,
        blob_hash: &str,
        status: &str,
        capture_id: Option<&str>,
        device_id: Option<&str>,
        error_code: Option<&str>,
    ) -> Result<crate::VoiceSyncTask> {
        let now = Utc::now().to_rfc3339();
        self.connection.execute(
            "INSERT INTO voice_sync_tasks(
                id,capture_id,device_id,direction,status,blob_hash,error_code,created_at,updated_at,row_version
             ) VALUES(?1,?2,?3,'download',?4,?5,?6,?7,?7,1)
             ON CONFLICT(blob_hash) DO UPDATE SET
                capture_id=excluded.capture_id,
                device_id=excluded.device_id,
                status=excluded.status,
                error_code=excluded.error_code,
                updated_at=excluded.updated_at,
                row_version=voice_sync_tasks.row_version+1",
            params![blob_hash, capture_id, device_id, status, blob_hash, error_code, now],
        )?;
        let task = self.connection.query_row(
            "SELECT id,capture_id,device_id,direction,status,blob_hash,error_code,created_at,updated_at,row_version
             FROM voice_sync_tasks WHERE blob_hash=?1",
            [blob_hash],
            |row| {
                Ok(crate::VoiceSyncTask {
                    id: row.get(0)?,
                    capture_id: row.get(1)?,
                    device_id: row.get(2)?,
                    direction: row.get(3)?,
                    status: row.get(4)?,
                    blob_hash: row.get(5)?,
                    error_code: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    row_version: row.get(9)?,
                })
            },
        )?;
        Ok(task)
    }
}

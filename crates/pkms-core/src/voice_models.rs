//! Voice-capture and ASR configuration contracts. Audio bytes never enter these DTOs.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const VOICE_MAX_TRANSCRIPT_CHARS: usize = 200_000;
pub const VOICE_MAX_LOCAL_DURATION_MS: u32 = 30 * 60 * 1000;
pub const VOICE_MAX_IMPORT_DURATION_MS: u32 = 4 * 60 * 60 * 1000;
pub const VOICE_MAX_BYTES: usize = 256 * 1024 * 1024;
pub const VOICE_MAX_LIST_ITEMS: u32 = 200;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum VoiceCaptureStatus {
    /// Audio is stored; no usable transcript exists yet.
    Pending,
    Transcribing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum VoiceTranscriptSource {
    None,
    Asr,
    Manual,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSourceKind {
    Local,
    Import,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct VoiceSegment {
    pub start_ms: u32,
    pub end_ms: u32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub speaker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct VoiceCapture {
    pub id: String,
    pub title: String,
    pub status: VoiceCaptureStatus,
    pub audio_blob_hash: String,
    pub audio_media_type: String,
    pub audio_bytes: u32,
    pub duration_ms: u32,
    pub transcript: String,
    pub segments: Vec<VoiceSegment>,
    pub transcript_source: VoiceTranscriptSource,
    pub source_kind: VoiceSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub device_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub row_version: u32,
    pub trashed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct CreateVoiceCapture {
    #[serde(default)]
    pub title: String,
    pub duration_ms: u32,
    pub source_kind: VoiceSourceKind,
    #[serde(default)]
    pub device_label: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdateVoiceCapture {
    pub capture_id: String,
    pub base_row_version: u32,
    pub title: String,
    pub transcript: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct VoiceCaptureTrashRequest {
    pub capture_id: String,
    pub base_row_version: u32,
    pub trashed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct SaveVoiceCaptureNote {
    pub capture_id: String,
    pub base_row_version: u32,
    #[serde(default)]
    #[ts(optional)]
    pub collection_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub base_collection_row_version: Option<u32>,
    #[serde(default)]
    pub tag_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct VoiceCapturePage {
    pub items: Vec<VoiceCapture>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct VoiceBrowseRequest {
    pub limit: u32,
    pub page: u32,
    pub trashed: bool,
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct VoiceBrowsePage {
    pub items: Vec<VoiceCapture>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
    pub next_cursor: Option<String>,
}

/// Workspace-level ASR endpoint. The credential itself is machine-local.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct AsrSettings {
    pub endpoint: String,
    pub model: String,
    pub language: String,
    pub row_version: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdateAsrSettings {
    pub base_row_version: u32,
    pub endpoint: String,
    pub model: String,
    pub language: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSummaryMode {
    Summary,
    Actions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct VoiceSummaryRequest {
    pub capture_id: String,
    pub mode: VoiceSummaryMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct VoiceSummary {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct VoiceSyncTask {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capture_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub device_id: Option<String>,
    pub direction: String,
    pub status: String,
    pub blob_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub row_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct AsrSettingsState {
    pub settings: AsrSettings,
    pub credential_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct AsrTestResult {
    pub latency_ms: u32,
    /// Provider text is deliberately not returned; an empty test tone often has none.
    pub accepted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_are_bounded_and_statuses_are_explicit() {
        assert_eq!(VOICE_MAX_LOCAL_DURATION_MS, 1_800_000);
        const { assert!(VOICE_MAX_IMPORT_DURATION_MS > VOICE_MAX_LOCAL_DURATION_MS) };
        assert_eq!(VOICE_MAX_LIST_ITEMS, 200);
        assert_ne!(VoiceCaptureStatus::Pending, VoiceCaptureStatus::Failed);
    }
}

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Transient handle created only through native file selection, drop or argv.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LocalDocument {
    pub token: String,
    pub display_name: String,
    pub location_hint: String,
    pub format: pkms_core::SourceFormat,
    #[ts(type = "number")]
    pub size: u64,
    pub resource_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LocalDocumentFailure {
    pub display_name: String,
    pub error: crate::ApiError,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LocalDocuments {
    pub documents: Vec<LocalDocument>,
    pub failures: Vec<LocalDocumentFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceIdentity {
    pub local_instance_id: String,
    pub workspace_id: String,
}

/// Why a recent workspace can or cannot be opened again. The interface maps each
/// reason to one sentence, so "the folder is gone" and "the folder now holds a
/// different library" never read the same.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAvailability {
    Available,
    /// The folder or its `library.sqlite3` is missing.
    Missing,
    /// The folder exists but carries another workspace's library.
    ForeignLibrary,
    /// The folder exists but cannot be read (permissions, offline volume).
    Unreadable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct WorkspaceEntry {
    pub identity: WorkspaceIdentity,
    pub path: String,
    pub last_opened_at: String,
    pub availability: WorkspaceAvailability,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

fn default_object_type_registry() -> String {
    r#"{"schema":1,"workspaces":{}}"#.into()
}

fn default_note_template_registry() -> String {
    r#"{"schema":1,"workspaces":{}}"#.into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct AppPreferences {
    #[serde(default)]
    pub language: Language,
    pub theme: Theme,
    /// Interface font size in logical pixels; 12..=24.
    pub ui_font_size: u16,
    /// Note body font size in logical pixels; 14..=32.
    pub editor_font_size: u16,
    /// Window zoom as a percentage of the native scale. The bounds match the
    /// zoom steps the workspace actually offers (50%..=300%).
    pub zoom_percent: u16,
    pub new_note_shortcut: String,
    pub search_shortcut: String,
    /// Versioned, workspace-keyed custom object type definitions.
    #[serde(default = "default_object_type_registry")]
    pub object_type_registry: String,
    /// Versioned, workspace-keyed note template definitions.
    #[serde(default = "default_note_template_registry")]
    pub note_template_registry: String,
}
impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            language: Language::default(),
            theme: Theme::System,
            ui_font_size: 14,
            editor_font_size: 17,
            zoom_percent: 100,
            new_note_shortcut: "Ctrl+N".into(),
            search_shortcut: "Ctrl+K".into(),
            object_type_registry: default_object_type_registry(),
            note_template_registry: default_note_template_registry(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
pub enum Language {
    #[default]
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
    #[serde(rename = "en")]
    English,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct PreferencesState {
    pub state_version: u32,
    pub preferences: AppPreferences,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum TabKind {
    Note,
    Source,
    Topic,
    Answer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct OpenTab {
    pub object_id: String,
    pub kind: TabKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ScrollOffset {
    pub object_id: String,
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct WindowGeometry {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub maximized: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct PaneState {
    #[serde(default)]
    pub scroll_offsets: Vec<ScrollOffset>,
    pub tabs: Vec<OpenTab>,
    pub active_object_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum Navigation {
    #[default]
    Inbox,
    Knowledge,
    Notes,
    Graph,
    Sources,
    Topics,
    Search,
    Review,
    Tasks,
    Trash,
    Settings,
    Applications,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct SessionLayout {
    #[serde(default)]
    pub window: Option<WindowGeometry>,
    pub panes: Vec<PaneState>,
    pub active_pane: u8,
    pub navigation: Navigation,
    pub sidebar_visible: bool,
    /// Preferred sidebar width in logical pixels; None keeps the UI default.
    #[serde(default)]
    pub sidebar_width: Option<u32>,
    /// Preferred note-list width in logical pixels; viewport fitting does not change it.
    #[serde(default)]
    pub note_list_width: Option<u32>,
}
impl Default for SessionLayout {
    fn default() -> Self {
        Self {
            window: None,
            panes: vec![PaneState::default()],
            active_pane: 0,
            navigation: Navigation::Inbox,
            sidebar_visible: true,
            sidebar_width: None,
            note_list_width: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct SessionState {
    pub identity: WorkspaceIdentity,
    pub state_version: u32,
    pub layout: SessionLayout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct RecentObject {
    pub object_id: String,
    pub last_opened_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum GrantPurpose {
    OpenWorkspace,
    CreateWorkspace,
    ImportFile,
    ImportSource,
    /// A user-selected BibTeX or CSL-JSON file imported as reference notes.
    ImportBibliography,
    /// A user-selected directory scanned for importable documents. Files found
    /// under it are re-checked against this root before any per-file grant.
    ImportDirectory,
    ExportFile,
    BackupInput,
    HandoffInput,
    HandoffOutput,
    RestoreWorkspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ResourceGrant {
    pub identity: Option<WorkspaceIdentity>,
    pub token: String,
    pub purpose: GrantPurpose,
    pub display_name: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    CreateWorkspace,
    OpenWorkspace,
    VerifyBackup,
    RestoreBackup,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct BootstrapAccepted {
    pub command_id: String,
    pub operation_id: String,
    pub replayed: bool,
}

/// One page of the app operation list. The cursor is opaque and bound to this
/// machine''s app state, so a page cannot be replayed into another catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct AppOperationPage {
    pub items: Vec<AppOperation>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct AppOperation {
    pub id: String,
    pub kind: OperationKind,
    pub status: OperationStatus,
    pub stage: String,
    pub result: Option<WorkspaceEntry>,
    pub report: Option<pkms_core::BackupReport>,
    pub error_code: Option<String>,
    pub cancel_requested: bool,
    pub active_in_current_process: bool,
    pub updated_at: String,
}

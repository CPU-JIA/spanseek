use crate::{MetadataReceipt, NotePage, NoteSummary, NotesFilter, ObjectMetadata};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum NoteSort {
    #[default]
    UpdatedDesc,
    UpdatedAsc,
    TitleAsc,
    TitleDesc,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum ViewLayout {
    List,
    Table,
    Board,
    Calendar,
    Timeline,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, TS)]
#[serde(rename_all = "snake_case")]
pub enum ViewProperty {
    InboxState,
    Tags,
    Archived,
    SearchEnabled,
    UpdatedAt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum BoardGroupBy {
    InboxState,
    Tag,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum ViewPropertyKind {
    Text,
    Number,
    Boolean,
    Date,
    Datetime,
    Null,
    List,
    Nested,
}

/// A bounded, read-only projection of one top-level YAML frontmatter key. The
/// original YAML remains the source of truth in the note document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ViewNoteProperty {
    pub key: String,
    pub value: String,
    pub kind: ViewPropertyKind,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ViewConfig {
    /// Ordered columns/card properties. The same bounded metadata is shown in
    /// every layout; it never changes the note body.
    pub visible_properties: Vec<ViewProperty>,
    pub board_group_by: BoardGroupBy,
    /// First visible calendar month in `YYYY-MM`; null means the current month.
    pub calendar_month: Option<String>,
    /// Top-level YAML frontmatter keys shown after the built-in properties.
    #[serde(default)]
    pub custom_properties: Vec<String>,
    /// When set, calendar and timeline use this date property instead of the
    /// note's last-edited timestamp. It must also be in `custom_properties`.
    #[serde(default)]
    pub date_property: Option<String>,
}

impl Default for ViewConfig {
    fn default() -> Self {
        Self {
            visible_properties: vec![
                ViewProperty::InboxState,
                ViewProperty::Tags,
                ViewProperty::UpdatedAt,
            ],
            board_group_by: BoardGroupBy::InboxState,
            calendar_month: None,
            custom_properties: vec![],
            date_property: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct SavedView {
    pub id: String,
    pub name: String,
    pub filter_schema: u32,
    /// Schema 1 is a conjunction of supported note filters, never SQL.
    pub filter: NotesFilter,
    pub sort: NoteSort,
    pub layout: ViewLayout,
    pub config_schema: u32,
    pub config: ViewConfig,
    pub is_pinned: bool,
    pub pin_position: Option<u32>,
    pub row_version: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct SaveView {
    pub view_id: Option<String>,
    pub base_row_version: Option<u32>,
    pub name: String,
    pub filter_schema: u32,
    pub filter: NotesFilter,
    pub sort: NoteSort,
    pub layout: ViewLayout,
    pub config_schema: u32,
    #[serde(default)]
    pub config: ViewConfig,
    pub is_pinned: bool,
    pub pin_position: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct RemoveView {
    pub view_id: String,
    pub base_row_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ViewCommit {
    /// The committed or removed view snapshot remains available on retry.
    pub view: SavedView,
    pub removed: bool,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ViewPageRequest {
    pub limit: u32,
    pub cursor: Option<String>,
    #[serde(default)]
    pub pinned_only: bool,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ViewPage {
    pub items: Vec<SavedView>,
    pub next_cursor: Option<String>,
}

/// A view result carries the immutable note summary and the current bounded
/// metadata needed by the property controls. The note body stays in its own
/// read path and is never copied into the view.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ViewNote {
    pub note: NoteSummary,
    pub metadata: ObjectMetadata,
    pub properties: Vec<ViewNoteProperty>,
    pub properties_truncated: bool,
    pub properties_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ViewNotePage {
    pub items: Vec<ViewNote>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ViewRun {
    pub view: SavedView,
    pub notes: ViewNotePage,
}

impl From<NotePage> for ViewNotePage {
    fn from(page: NotePage) -> Self {
        Self {
            items: page
                .items
                .into_iter()
                .map(|note| ViewNote {
                    metadata: ObjectMetadata {
                        id: note.id.clone(),
                        kind: crate::ObjectKind::Note,
                        row_version: note.row_version,
                        archived: note.archived,
                        trashed: note.trashed,
                        inbox_state: note.inbox_state,
                        search_enabled: note.search_enabled,
                        tags: Vec::new(),
                    },
                    note,
                    properties: Vec::new(),
                    properties_truncated: false,
                    properties_error: false,
                })
                .collect(),
            next_cursor: page.next_cursor,
        }
    }
}

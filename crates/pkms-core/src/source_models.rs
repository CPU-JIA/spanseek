use crate::CleaningReport;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Text,
    Markdown,
    Pdf,
    Html,
    /// Office Open XML document. Parsed by the worker's own extractor; the
    /// original container is stored unchanged.
    Docx,
    /// Legacy Word 97-2003 binary document. Parsed by the worker's bounded
    /// binary reader; the original container is stored unchanged.
    Doc,
    /// EPUB package. The worker reads the spine order and keeps the container.
    Epub,
    /// Amazon MOBI/PalmDOC book. The worker reads uncompressed and PalmDOC text.
    Mobi,
    /// Comma-separated values. Each row becomes one text block.
    Csv,
    /// Office Open XML workbook. Cells are read as text rows per sheet.
    Xlsx,
    /// Legacy BIFF Excel workbook. Parsed by the worker's bounded binary reader.
    Xls,
    /// Office Open XML presentation. Slide text is read in slide order.
    Pptx,
}
impl SourceFormat {
    /// Every format this build can store, in a stable order. Statistics and
    /// exports derive their vocabulary from this list, so a format added here
    /// can never make an existing inventory fail as an unknown key.
    pub const ALL: [SourceFormat; 12] = [
        Self::Text,
        Self::Markdown,
        Self::Pdf,
        Self::Html,
        Self::Docx,
        Self::Doc,
        Self::Epub,
        Self::Mobi,
        Self::Csv,
        Self::Xlsx,
        Self::Xls,
        Self::Pptx,
    ];
    /// The exact value stored in `source_versions.format`.
    pub fn stored(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Markdown => "markdown",
            Self::Pdf => "pdf",
            Self::Html => "html",
            Self::Docx => "docx",
            Self::Doc => "doc",
            Self::Epub => "epub",
            Self::Mobi => "mobi",
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
            Self::Xls => "xls",
            Self::Pptx => "pptx",
        }
    }
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Text => "text/plain",
            Self::Markdown => "text/markdown",
            Self::Pdf => "application/pdf",
            Self::Html => "text/html",
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Doc => "application/msword",
            Self::Epub => "application/epub+zip",
            Self::Mobi => "application/x-mobipocket-ebook",
            Self::Csv => "text/csv",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Xls => "application/vnd.ms-excel",
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ImportSource {
    pub input_token: String,
    pub original_name: String,
    pub format: SourceFormat,
    pub origin_url: Option<String>,
    pub collection_id: Option<String>,
    pub add_version_to: Option<String>,
    pub base_row_version: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ImportWebpage {
    pub url: String,
    pub collection_id: Option<String>,
    pub add_version_to: Option<String>,
    pub base_row_version: Option<u32>,
}
/// The exact immutable UTF-8 original, before any search-oriented parsing.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceNoteInput {
    pub version: SourceVersion,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CreateSourceNote {
    pub source_version_id: String,
    pub source_blob_hash: String,
    pub title: String,
    pub document_schema: u32,
    pub document: crate::NoteDocument,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WebCaptureInput {
    pub final_url: String,
    pub http_status: u16,
    pub content_type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WebCapture {
    pub final_url: String,
    pub http_status: u16,
    pub content_type: String,
    pub captured_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceVersion {
    #[serde(default)]
    pub web_capture: Option<WebCapture>,
    pub id: String,
    pub source_id: String,
    pub blob_hash: String,
    pub original_name: String,
    pub origin_url: Option<String>,
    pub captured_at: String,
    pub format: SourceFormat,
    #[ts(type = "number")]
    pub size: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SourceSummary {
    pub id: String,
    pub display_title: String,
    pub current_version_id: String,
    pub row_version: u32,
    pub metadata_revision: u32,
    pub trashed: bool,
    pub format: SourceFormat,
    pub text_ready: bool,
    pub lexical_ready: bool,
    pub active_parse_id: Option<String>,
    pub updated_at: String,
    #[serde(default)]
    pub origin_url: Option<String>,
    #[serde(default)]
    pub original_name: String,
    #[serde(default)]
    pub content_hash: String,
    #[serde(default)]
    pub preview_text: Option<String>,
    /// The document's own author when the file carries one, otherwise None;
    /// the interface shows "not provided" instead of inventing a name.
    #[serde(default)]
    pub author: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourcePage {
    pub items: Vec<SourceSummary>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourcesPageRequest {
    #[serde(default)]
    pub filter: Option<SourcesFilter>,
    pub limit: u32,
    pub cursor: Option<String>,
    pub trashed: bool,
    pub query: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SourcesBrowseRequest {
    pub page: u32,
    pub limit: u32,
    pub trashed: bool,
    pub query: Option<String>,
    pub filter: Option<SourcesFilter>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourcesBrowsePage {
    pub items: Vec<SourceSummary>,
    pub total: u32,
    pub page: u32,
    pub limit: u32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SourcesFilter {
    pub format: Option<SourceFormat>,
    pub reading_status: Option<ReadingStatus>,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    pub inbox_state: Option<crate::InboxState>,
    pub collection_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceBlock {
    pub id: String,
    pub text: String,
    pub page_index: Option<u32>,
    #[serde(default)]
    pub pdf_geometry: Option<PdfBlockGeometry>,
}
/// A table reconstructed from stored character geometry. `rows` are raw cell
/// strings in reading order; the original block text is never replaced.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceTable {
    pub page_index: u32,
    pub rows: Vec<Vec<String>>,
    pub bbox: [f64; 4],
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PdfCharacterBox {
    pub start_codepoint: u32,
    pub end_codepoint: u32,
    pub rect: Option<[f64; 4]>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PdfBlockGeometry {
    pub page_label: Option<String>,
    pub crop_box: [f64; 4],
    pub rotation: u16,
    pub coordinate_space: String,
    pub chars: Vec<PdfCharacterBox>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SourceTocItem {
    pub title: String,
    pub level: u32,
    #[serde(default)]
    pub block_id: Option<String>,
    #[serde(default)]
    pub block_position: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ParsedSourceImage {
    pub block_position: u32,
    pub media_type: String,
    pub data_base64: String,
    #[serde(default)]
    pub alt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceBlockImage {
    pub block_id: String,
    pub blob_hash: String,
    pub media_type: String,
    #[serde(default)]
    pub alt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceImageData {
    pub source_version_id: String,
    pub parse_id: String,
    pub blob_hash: String,
    pub media_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ParsedCover {
    pub media_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceCoverData {
    pub source_version_id: String,
    pub parse_id: String,
    pub media_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ParsedSource {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    pub source_version_id: String,
    pub parser_version: String,
    pub blocks: Vec<SourceBlock>,
    pub page_count: Option<u32>,
    pub needs_ocr: bool,
    #[serde(default)]
    pub toc: Vec<SourceTocItem>,
    #[serde(default)]
    pub cover: Option<ParsedCover>,
    #[serde(default)]
    pub images: Vec<ParsedSourceImage>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceTextPage {
    pub source_version_id: String,
    pub parse_id: String,
    pub blocks: Vec<SourceBlock>,
    pub page_count: Option<u32>,
    /// What the cleaning step removed for this parse, when it ran.
    #[serde(default)]
    pub cleaning: Option<CleaningReport>,
    pub needs_ocr: bool,
    #[serde(default)]
    pub toc: Vec<SourceTocItem>,
    #[serde(default)]
    pub has_cover: bool,
    #[serde(default)]
    pub images: Vec<SourceBlockImage>,
    /// True when at least one page without a text layer was OCR'd.
    #[serde(default)]
    pub ocr_applied: bool,
    /// Stable engine label when OCR ran; absent otherwise.
    #[serde(default)]
    pub ocr_engine: Option<String>,
    /// Best-effort tables reconstructed from character geometry on this page.
    #[serde(default)]
    pub tables: Vec<SourceTable>,
    pub next_offset: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceJob {
    pub id: String,
    pub input: ImportSource,
    #[serde(default)]
    pub tag_names: Vec<String>,
    pub status: crate::JobStatus,
    pub stage: String,
    pub attempt: u32,
    pub config_revision: u32,
    pub cancel_requested: bool,
    pub source_id: Option<String>,
    pub source_version_id: Option<String>,
    pub error_code: Option<String>,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceJobLease {
    pub job: SourceJob,
    pub lease_token: String,
    pub expires_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceJobEvent {
    pub attempt: Option<u32>,
    pub seq: u32,
    pub status: crate::JobStatus,
    pub stage: String,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceJobPage {
    pub items: Vec<SourceJob>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceLocator {
    Text {
        source_version_id: String,
        block_id: String,
        start_codepoint: u32,
        end_codepoint: u32,
        quote: String,
        prefix: String,
        suffix: String,
    },
    Pdf {
        source_version_id: String,
        page_index: u32,
        page_label: Option<String>,
        crop_box: [f64; 4],
        rotation: u16,
        coordinate_space: String,
        rects: Vec<[f64; 4]>,
        quote: String,
        prefix: String,
        suffix: String,
    },
}
impl SourceLocator {
    pub fn source_version_id(&self) -> &str {
        match self {
            Self::Text {
                source_version_id, ..
            }
            | Self::Pdf {
                source_version_id, ..
            } => source_version_id,
        }
    }
    pub fn quote(&self) -> &str {
        match self {
            Self::Text { quote, .. } | Self::Pdf { quote, .. } => quote,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Annotation {
    pub id: String,
    pub source_version_id: String,
    pub locator_schema: u32,
    pub locator: SourceLocator,
    pub comment: String,
    pub row_version: u32,
    pub trashed: bool,
}
/// Bounded list projection; resolve the annotation ID to read its full locator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AnnotationSummary {
    pub id: String,
    pub source_id: String,
    pub source_title: String,
    pub source_version_id: String,
    pub quote_preview: String,
    pub comment_preview: String,
    pub row_version: u32,
    pub trashed: bool,
    pub source_trashed: bool,
    pub source_search_enabled: bool,
    pub source_format: SourceFormat,
    pub page_index: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AnnotationsQueryRequest {
    pub limit: u32,
    pub cursor: Option<String>,
    pub query: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnnotationPage {
    pub items: Vec<AnnotationSummary>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateAnnotation {
    pub locator: SourceLocator,
    pub comment: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct UpdateAnnotation {
    pub id: String,
    pub base_row_version: u32,
    pub comment: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LocatorStatus {
    Exact,
    Approximate,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ResolvedAnnotation {
    pub annotation: Annotation,
    pub status: LocatorStatus,
    pub reason: Option<String>,
    pub parse_id: Option<String>,
    pub block_offset: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReadingStatus {
    Unread,
    Reading,
    Finished,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReadingState {
    pub source_id: String,
    pub source_version_id: String,
    pub locator: Option<SourceLocator>,
    pub status: ReadingStatus,
    pub row_version: u32,
    #[serde(default)]
    pub parse_id: Option<String>,
    #[serde(default)]
    pub block_offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub pdf_view: Option<PdfReadingView>,
}
/// The user's last PDF scaling intent. Fit modes are recomputed after a
/// window, sidebar or focus-mode resize; custom keeps the explicit percentage.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum PdfZoomMode {
    #[default]
    Width,
    Page,
    Custom,
}
/// Normalized coordinates in the unrotated PDF crop box, independent of zoom.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct PdfReadingView {
    pub page_index: u32,
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub scale: f64,
    /// Additional clockwise rotation relative to the PDF's intrinsic rotation.
    pub rotation: u16,
    /// Legacy rows predate this field; width is the responsive default.
    #[serde(default)]
    pub zoom_mode: PdfZoomMode,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SetReadingState {
    pub source_id: String,
    pub source_version_id: String,
    pub locator: Option<SourceLocator>,
    pub status: ReadingStatus,
    pub base_row_version: u32,
    // Omission preserves the exact pre-K17 command hash for persisted retries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub pdf_view: Option<PdfReadingView>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct UpdateSourceMetadata {
    pub source_id: String,
    pub base_row_version: u32,
    pub display_title: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceMutation {
    pub source: Option<SourceSummary>,
    pub annotation: Option<Annotation>,
    pub reading: Option<ReadingState>,
    pub receipt: crate::MetadataReceipt,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceSearchHit {
    pub source_id: String,
    pub source_version_id: String,
    pub block_id: String,
    pub title: String,
    pub text: String,
    pub page_index: Option<u32>,
    pub bm25: f64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SourceExportReport {
    pub source_id: String,
    pub source_version_id: String,
    pub original_path: String,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceResourceTicket {
    pub url: String,
    #[ts(type = "number")]
    pub size: u64,
    pub media_type: String,
    pub expires_at: String,
}
impl crate::mutations::RecordedResult for SourceMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}

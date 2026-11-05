use pkms_worker::OcrConfig;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What the local OCR engine can do right now. The engine never comes from an
/// imported document, so this is an installation fact the interface can show.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct OcrStatus {
    pub available: bool,
    /// Why OCR is unavailable; stable code, never a localized sentence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tessdata_dir: Option<String>,
    /// Language packs present next to the engine (`chi_sim`, `eng`, ...).
    pub languages: Vec<String>,
    /// The `languages` argument handed to Tesseract for scanned pages.
    pub configured_languages: String,
    /// Rasterization DPI used only for pages without a text layer.
    pub dpi: u32,
}

/// `languages` is the list found on disk; it is reported even when a requested
/// pack is missing so the interface can point at the gap.
pub fn ocr_status_from(config: Option<OcrConfig>, languages: Vec<String>) -> OcrStatus {
    match config {
        Some(config) => OcrStatus {
            available: true,
            reason: None,
            executable: Some(config.executable.display().to_string()),
            tessdata_dir: Some(config.tessdata_dir.display().to_string()),
            languages,
            configured_languages: config.languages,
            dpi: config.dpi,
        },
        None => OcrStatus {
            available: false,
            reason: Some("ocr_not_installed".into()),
            executable: None,
            tessdata_dir: None,
            languages,
            configured_languages: "chi_sim+eng".into(),
            dpi: 300,
        },
    }
}

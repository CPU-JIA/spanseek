//! OCR capability reporting: installation facts only, never document content.
use pkms_app::{OcrConfig, ocr_status_from};
use std::path::PathBuf;

#[test]
fn a_missing_engine_reports_a_stable_reason_and_the_requested_languages() {
    let status = ocr_status_from(None, vec!["eng".into()]);
    assert!(!status.available);
    assert_eq!(status.reason.as_deref(), Some("ocr_not_installed"));
    assert_eq!(status.configured_languages, "chi_sim+eng");
    assert_eq!(status.languages, ["eng"]);
    assert_eq!(status.dpi, 300);
    assert!(status.executable.is_none());
    assert!(status.tessdata_dir.is_none());
}

#[test]
fn an_installed_engine_exposes_its_paths_and_language_packs() {
    let status = ocr_status_from(
        Some(OcrConfig {
            executable: PathBuf::from("C:/ocr/tesseract.exe"),
            tessdata_dir: PathBuf::from("C:/ocr/tessdata"),
            languages: "chi_sim+eng".into(),
            dpi: 300,
        }),
        vec!["chi_sim".into(), "eng".into()],
    );
    assert!(status.available);
    assert!(status.reason.is_none());
    assert_eq!(status.languages, ["chi_sim", "eng"]);
    assert!(status.executable.unwrap().ends_with("tesseract.exe"));
    assert!(status.tessdata_dir.unwrap().ends_with("tessdata"));
}

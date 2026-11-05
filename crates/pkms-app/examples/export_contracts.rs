use pkms_app::*;
use ts_rs::{Config, TS};

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .ok_or("usage: export_contracts <output-directory>")?;
    let config = Config::default().with_out_dir(output);
    WorkspaceEntry::export_all(&config)?;
    PreferencesState::export_all(&config)?;
    SessionState::export_all(&config)?;
    RecentObject::export_all(&config)?;
    ResourceGrant::export_all(&config)?;
    BootstrapAccepted::export_all(&config)?;
    AppOperation::export_all(&config)?;
    DesktopRequest::export_all(&config)?;
    DesktopReply::export_all(&config)?;
    GenerationCredentialStatus::export_all(&config)?;
    ApiError::export_all(&config)?;
    LocalDocuments::export_all(&config)?;
    DesktopEvent::export_all(&config)?;
    OcrStatus::export_all(&config)?;
    Ok(())
}

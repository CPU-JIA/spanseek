//! Small, locally prepared diagnostic archives. The App owns content redaction.
use crate::{
    DiagnosticCategory, DiagnosticExportReport, DiagnosticFile, Error, PreparedOutput, Result,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// Accepts only fixed categories. The App must build their field-whitelisted payloads.
pub fn prepare_diagnostics_archive(
    destination: &Path,
    files: Vec<(DiagnosticCategory, Vec<u8>)>,
    check: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<PreparedOutput<DiagnosticExportReport>> {
    if files.is_empty() || files.len() > 3 {
        return Err(Error::InvalidInput("invalid diagnostic categories"));
    }
    let mut names = BTreeSet::new();
    let mut payloads = Vec::new();
    let mut reports = Vec::new();
    for (category, bytes) in files {
        check("validate_diagnostics")?;
        let name = match category {
            DiagnosticCategory::Versions => "versions.json",
            DiagnosticCategory::Statistics => "statistics.json",
            DiagnosticCategory::Errors => "errors.json",
        };
        if !names.insert(name) || bytes.len() > 1024 * 1024 {
            return Err(Error::InvalidInput("invalid diagnostic payload"));
        }
        match category {
            DiagnosticCategory::Versions => {
                let _: crate::DiagnosticVersions = serde_json::from_slice(&bytes)?;
            }
            DiagnosticCategory::Statistics => {
                let _: crate::WorkspaceStatistics = serde_json::from_slice(&bytes)?;
            }
            DiagnosticCategory::Errors => {
                let _: Vec<crate::DiagnosticError> = serde_json::from_slice(&bytes)?;
            }
        }
        reports.push(DiagnosticFile {
            name: name.into(),
            sha256: crate::portable::hash_bytes(&bytes),
            bytes: bytes.len() as u64,
        });
        payloads.push((name, bytes));
    }
    let bundle_id = uuid::Uuid::new_v4().to_string();
    let generated_at = chrono::Utc::now().to_rfc3339();
    let manifest = serde_json::to_vec_pretty(
        &serde_json::json!({"format_version":1,"bundle_id":bundle_id,"generated_at":generated_at,"files":reports}),
    )?;
    reports.push(DiagnosticFile {
        name: "manifest.json".into(),
        sha256: crate::portable::hash_bytes(&manifest),
        bytes: manifest.len() as u64,
    });
    let mut output = crate::portable::new_output(destination)?;
    {
        let mut writer = zip::ZipWriter::new(output.as_file_mut());
        for (name, bytes) in payloads {
            check("write_diagnostics")?;
            crate::portable::zip_bytes(&mut writer, name, &bytes)?;
        }
        crate::portable::zip_bytes(&mut writer, "manifest.json", &manifest)?;
        writer.finish().map_err(crate::portable::archive_error)?;
    }
    output.as_file().sync_all()?;
    output.as_file_mut().seek(SeekFrom::Start(0))?;
    let sha256 = hash_archive(output.as_file_mut(), check)?;
    check("diagnostics_prepared")?;
    Ok(PreparedOutput {
        file: output,
        destination: destination.into(),
        report: DiagnosticExportReport {
            bundle_id,
            generated_at,
            sha256,
            files: reports,
        },
    })
}

fn hash_archive(file: &mut File, check: &mut dyn FnMut(&str) -> Result<()>) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        check("hash_diagnostics")?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

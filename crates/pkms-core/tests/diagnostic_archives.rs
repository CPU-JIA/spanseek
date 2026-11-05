use pkms_core::*;
use sha2::{Digest, Sha256};
use std::{fs, io::Read};

fn payload() -> Vec<u8> {
    serde_json::to_vec(&DiagnosticVersions {
        application: "test".into(),
        app_schema: 3,
        knowledge_schema: 13,
        platform: "windows".into(),
        architecture: "x86_64".into(),
        webview_version: None,
        source_worker_protocol: 1,
        inference_protocol: 1,
        embedding_model_revision: "test".into(),
    })
    .unwrap()
}

#[test]
fn diagnostic_archive_has_fixed_names_manifest_and_verified_hashes() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("diagnostics.zip");
    let prepared = prepare_diagnostics_archive(
        &output,
        vec![(DiagnosticCategory::Versions, payload())],
        &mut |_| Ok(()),
    )
    .unwrap();
    assert!(!output.exists());
    let report = prepared.publish().unwrap();
    let bytes = fs::read(&output).unwrap();
    assert_eq!(report.sha256, format!("{:x}", Sha256::digest(&bytes)));
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(zip.len(), 2);
    for file in &report.files {
        let mut bytes = Vec::new();
        zip.by_name(&file.name)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes.len() as u64, file.bytes);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), file.sha256);
    }
    assert!(
        prepare_diagnostics_archive(
            &output,
            vec![(DiagnosticCategory::Versions, payload())],
            &mut |_| Ok(())
        )
        .is_err()
    );
}

#[test]
fn invalid_duplicate_or_canceled_diagnostics_never_publish() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("diagnostics.zip");
    for files in [
        vec![],
        vec![
            (DiagnosticCategory::Versions, payload()),
            (DiagnosticCategory::Versions, payload()),
        ],
        vec![(
            DiagnosticCategory::Versions,
            br#"{"private_body":"must not be accepted"}"#.to_vec(),
        )],
        vec![(DiagnosticCategory::Errors, vec![b' '; 1024 * 1024 + 1])],
    ] {
        assert!(prepare_diagnostics_archive(&output, files, &mut |_| Ok(())).is_err());
        assert!(!output.exists());
    }
    assert!(
        prepare_diagnostics_archive(
            &output,
            vec![(DiagnosticCategory::Versions, payload())],
            &mut |stage| if stage == "write_diagnostics" {
                Err(Error::InvalidInput("canceled"))
            } else {
                Ok(())
            }
        )
        .is_err()
    );
    assert!(!output.exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

//! Explicit live smoke test: synthetic evidence only, secret JSON through stdin, sanitized output.
use pkms_app::{
    WorkspaceIdentity,
    inference_credentials::{self as vault, CredentialBinding, Secret},
    verify_remote_generation_model,
};
use pkms_core::GenerationProfile;
use serde::Deserialize;
use std::io::Read;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    profile: GenerationProfile,
    api_key: String,
}
struct Cleanup(CredentialBinding);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = vault::delete(&self.0);
    }
}
fn main() {
    let mut bytes = Vec::new();
    if std::io::stdin()
        .take(32 * 1024)
        .read_to_end(&mut bytes)
        .is_err()
    {
        println!("input_failed");
        std::process::exit(2);
    }
    let input: Input = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            bytes.fill(0);
            println!("input_invalid");
            std::process::exit(2);
        }
    };
    bytes.fill(0);
    let identity = WorkspaceIdentity {
        local_instance_id: uuid::Uuid::new_v4().to_string(),
        workspace_id: uuid::Uuid::new_v4().to_string(),
    };
    let result = (|| {
        pkms_core::validate_generation_profile(&input.profile)
            .map_err(|_| "invalid_profile".to_owned())?;
        let binding = CredentialBinding::new(
            &identity.local_instance_id,
            &identity.workspace_id,
            &input.profile.provider,
            &input.profile.endpoint,
            &input.profile.model,
        )
        .map_err(|_| "invalid_binding".to_owned())?;
        let cleanup = Cleanup(binding);
        let secret = Secret::new(input.api_key).map_err(|_| "invalid_key".to_owned())?;
        vault::store(&cleanup.0, &secret).map_err(|_| "credential_store_failed".to_owned())?;
        let result = verify_remote_generation_model(
            &identity,
            &input.profile,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .map_err(|error| match error {
            pkms_app::AppError::InvalidInput(code) => code.to_owned(),
            _ => "verification_failed".into(),
        });
        vault::delete(&cleanup.0).map_err(|_| "credential_cleanup_failed".to_owned())?;
        if vault::exists(&cleanup.0).map_err(|_| "credential_cleanup_failed".to_owned())? {
            return Err("credential_cleanup_failed".into());
        }
        result
    })();
    match result {
        Ok(()) => println!("verified_synthetic_answer; credential_removed"),
        Err(code) => {
            println!("failed: {code}");
            std::process::exit(1);
        }
    }
}

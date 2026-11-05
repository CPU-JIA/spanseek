//! Synthetic paired network timing: one-shot runtime/client vs retained runtime/pool.
use pkms_app::{
    WorkspaceIdentity,
    inference_credentials::{self as vault, CredentialBinding},
};
use pkms_inference::remote_embeddings::{self, Client};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Deserialize)]
struct Input {
    output: PathBuf,
    endpoint: String,
    credential_identity: WorkspaceIdentity,
    pairs: usize,
}
fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut b = Vec::new();
    std::io::stdin().take(16384).read_to_end(&mut b)?;
    let input: Input = serde_json::from_slice(&b)?;
    if input.pairs < 1 || input.pairs > 100 {
        return Err("pair count invalid".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&input.output)?;
    let binding = CredentialBinding::new(
        &input.credential_identity.local_instance_id,
        &input.credential_identity.workspace_id,
        "openai_embeddings",
        &input.endpoint,
        remote_embeddings::MODEL,
    )?;
    let secret = vault::load(&binding)?.ok_or("credential missing")?;
    let retained = runtime()?;
    let client = Client::new()?;
    let texts = ["合成测试：知识库每天晚上备份，副本保留七天。".to_owned()];
    for pair in 0..input.pairs {
        for reuse in if pair % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let start = Instant::now();
            let result = if reuse {
                retained.block_on(client.embed(
                    &input.endpoint,
                    secret.expose_secret(),
                    &texts,
                    Duration::from_secs(30),
                    || Ok(()),
                ))
            } else {
                runtime()?.block_on(remote_embeddings::embed(
                    &input.endpoint,
                    secret.expose_secret(),
                    &texts,
                    Duration::from_secs(30),
                    || Ok(()),
                ))
            };
            let row = json!({"pair":pair,"reuse":reuse,"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"success":result.is_ok(),"dimensions":result.as_ref().ok().and_then(|v|v.first()).map(Vec::len),"error":result.err().map(|e|e.0)});
            writeln!(file, "{row}")?;
            file.flush()?;
        }
        println!("pairs_completed={}", pair + 1);
    }
    Ok(())
}

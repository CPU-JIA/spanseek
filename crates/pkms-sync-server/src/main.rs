use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use clap::{Parser, Subcommand};
use pkms_sync::{SyncAuthority, hex_decode, validate_public_key};
use pkms_sync_server::router;
use serde_json::json;

#[derive(Debug, Parser)]
#[command(
    name = "pkms-sync-server",
    about = "spanseek sync authority HTTP server"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve signed versioned-protocol requests over HTTP.
    Serve {
        /// SQLite authority database path.
        #[arg(long)]
        db: PathBuf,
        /// Optional content-addressed blob root.
        #[arg(long)]
        blob_root: Option<PathBuf>,
        /// Listen address. Defaults to loopback only.
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
    },
    /// Enroll the first owner device before serving.
    ///
    /// The proof is the Ed25519 signature of `owner_proof_message(public_key,
    /// display_name)`, produced on the owner device. This command is local
    /// administration only and must not be exposed over the network.
    BootstrapOwner {
        #[arg(long)]
        db: PathBuf,
        /// Owner Ed25519 public key as lowercase hex.
        #[arg(long)]
        public_key: String,
        #[arg(long)]
        display_name: String,
        /// Ed25519 signature as lowercase hex.
        #[arg(long)]
        proof: String,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Serve {
            db,
            blob_root,
            bind,
        } => serve(db, blob_root, bind),
        Command::BootstrapOwner {
            db,
            public_key,
            display_name,
            proof,
        } => bootstrap_owner(db, &public_key, &display_name, &proof),
    }
}

fn open_authority(
    db: PathBuf,
    blob_root: Option<PathBuf>,
) -> Result<SyncAuthority, Box<dyn std::error::Error>> {
    match blob_root {
        Some(root) => Ok(SyncAuthority::open_with_blobs(db, root)?),
        None => Ok(SyncAuthority::open(db)?),
    }
}

fn bootstrap_owner(
    db: PathBuf,
    public_key: &str,
    display_name: &str,
    proof: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_public_key(public_key)?;
    let public_key = hex_decode(public_key).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "public key is not hex")
    })?;
    let proof = hex_decode(proof)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "proof is not hex"))?;
    let mut authority = SyncAuthority::open(db)?;
    let device = authority.enroll_owner(&public_key, display_name, &proof)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "device_id": device.device_id,
            "fingerprint": device.fingerprint,
            "display_name": device.display_name,
        }))?
    );
    Ok(())
}

fn serve(
    db: PathBuf,
    blob_root: Option<PathBuf>,
    bind: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let authority = open_authority(db, blob_root)?;
    let shared = Arc::new(Mutex::new(authority));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&bind).await?;
        let address = listener.local_addr()?;
        println!("pkms-sync-server listening on http://{address}");
        axum::serve(listener, router(shared)).await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;
    Ok(())
}

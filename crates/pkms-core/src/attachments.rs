use crate::{
    AttachmentImport, BlobInfo, Error, Library, NodeType, NoteDocument, Result,
    document::valid_uuid,
};
use chrono::Utc;
use image::{GenericImageView, ImageFormat, ImageReader, Limits};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Cursor, Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;

const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_SIDE: u32 = 16_384;
const MAX_PIXELS: u64 = 40_000_000;
const PREVIEW_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Serialize, Deserialize)]
struct ImportResult {
    blob: BlobInfo,
    original_name: String,
}

impl Library {
    /// The caller captures a user-selected file/clipboard before invoking this core API.
    /// Extensions are not trusted; only decoded PNG/JPEG/WebP bytes are accepted.
    pub fn import_attachment(
        &mut self,
        command_id: &str,
        original_name: &str,
        bytes: &[u8],
    ) -> Result<AttachmentImport> {
        valid_uuid(command_id)?;
        validate_name(original_name)?;
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return Err(Error::InvalidInput(
                "image must be between 1 byte and 20 MiB",
            ));
        }
        let blob_hash = hash(bytes);
        let payload_hash = hash(&serde_json::to_vec(&(
            1,
            "attachments.import",
            &blob_hash,
            original_name,
        ))?);
        if let Some((result, seq)) = self.replay_import(command_id, &payload_hash)? {
            self.read_blob(&result.blob)?;
            return Ok(self.import_response(command_id, result, seq, true));
        }
        let blob = decode_image(bytes, blob_hash)?;
        self.publish_blob(&blob, bytes)?;
        let result = ImportResult {
            blob,
            original_name: original_name.into(),
        };
        let result_json = serde_json::to_string(&result)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Recheck after capture: another adapter can have completed this command.
        let existing: Option<String> = tx
            .query_row(
                "SELECT payload_hash FROM command_receipts WHERE command_id=?1",
                [command_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(expected) = existing {
            if expected != payload_hash {
                return Err(Error::IdempotencyMismatch);
            }
            drop(tx);
            let (result, seq) = self
                .replay_import(command_id, &payload_hash)?
                .ok_or(Error::IntegrityFailure)?;
            return Ok(self.import_response(command_id, result, seq, true));
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO blobs(hash,size,media_type,width,height,created_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(hash) DO NOTHING",params![result.blob.hash,result.blob.size,result.blob.media_type,result.blob.width,result.blob.height,now])?;
        let seq: i64 = tx.query_row(
            "SELECT next_change_seq FROM workspace_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let next = seq
            .checked_add(1)
            .ok_or(Error::InvalidInput("change sequence overflow"))?;
        tx.execute(
            "UPDATE workspace_meta SET next_change_seq=?1 WHERE singleton=1",
            [next],
        )?;
        tx.execute("INSERT INTO command_receipts(command_id,payload_hash,command_kind,object_id,change_seq,committed_at,result_json) VALUES(?1,?2,'attachments.import',?3,?4,?5,?6)",params![command_id,payload_hash,result.blob.hash,seq,now,result_json])?;
        tx.commit()?;
        Ok(self.import_response(command_id, result, seq.to_string(), false))
    }

    pub(crate) fn store_image_bytes(&self, bytes: &[u8]) -> Result<BlobInfo> {
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return Err(Error::InvalidInput(
                "image must be between 1 byte and 20 MiB",
            ));
        }
        let blob = decode_image(bytes, hash(bytes))?;
        self.publish_blob(&blob, bytes)?;
        self.connection.execute("INSERT INTO blobs(hash,size,media_type,width,height,created_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(hash) DO NOTHING",params![blob.hash,blob.size,blob.media_type,blob.width,blob.height,Utc::now().to_rfc3339()])?;
        Ok(blob)
    }

    pub(crate) fn image_blob(&self, hash: &str) -> Result<(BlobInfo, Vec<u8>)> {
        let info = self.blob_info(hash)?;
        let bytes = self.read_blob(&info)?;
        Ok((info, bytes))
    }

    pub fn attachment_preview(&self, token: &str) -> Result<Vec<u8>> {
        self.attachment_preview_resource(token)
            .map(|(_, bytes)| bytes)
    }

    pub fn attachment_preview_resource(&self, token: &str) -> Result<(BlobInfo, Vec<u8>)> {
        let (hash, expires) = self.attachment_previews.get(token).ok_or(Error::NotFound)?;
        if Instant::now() >= *expires {
            return Err(Error::NotFound);
        }
        let info = self.blob_info(hash)?;
        let bytes = self.read_blob(&info)?;
        Ok((info, bytes))
    }

    /// Recover a committed import after restart without reopening its source file.
    /// The command receipt, never an arbitrary hash, authorizes a new preview.
    pub fn resume_attachment_import(&mut self, command_id: &str) -> Result<AttachmentImport> {
        valid_uuid(command_id)?;
        let (json, seq): (String, i64) = self.connection.query_row(
            "SELECT result_json,change_seq FROM command_receipts WHERE command_id=?1 AND command_kind='attachments.import'",
            [command_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?.ok_or(Error::NotFound)?;
        let result: ImportResult = serde_json::from_str(&json)?;
        self.read_blob(&result.blob)?;
        Ok(self.import_response(command_id, result, seq.to_string(), true))
    }

    /// Authorizes against the exact immutable revision, including historical or trashed notes.
    pub fn attachment(&self, revision_id: &str, blob_hash: &str) -> Result<(BlobInfo, Vec<u8>)> {
        valid_uuid(revision_id)?;
        validate_hash(blob_hash)?;
        let referenced:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM note_revision_blobs WHERE revision_id=?1 AND blob_hash=?2)",params![revision_id,blob_hash],|r|r.get(0))?;
        if !referenced {
            return Err(Error::NotFound);
        }
        let info = self.blob_info(blob_hash)?;
        let bytes = self.read_blob(&info)?;
        Ok((info, bytes))
    }

    pub(crate) fn verify_document_attachments(&self, document: &NoteDocument) -> Result<()> {
        let hashes: BTreeSet<_> = document
            .nodes()
            .iter()
            .filter(|n| n.kind == NodeType::Image)
            .filter_map(|n| n.attrs.blob_hash.as_deref())
            .collect();
        for hash in hashes {
            self.read_blob(&self.blob_info(hash)?)?;
        }
        Ok(())
    }

    fn replay_import(
        &self,
        command_id: &str,
        hash: &str,
    ) -> Result<Option<(ImportResult, String)>> {
        let row=self.connection.query_row("SELECT payload_hash,result_json,change_seq FROM command_receipts WHERE command_id=?1",[command_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,i64>(2)?))).optional()?;
        let Some((expected, result, seq)) = row else {
            return Ok(None);
        };
        if expected != hash {
            return Err(Error::IdempotencyMismatch);
        }
        Ok(Some((
            serde_json::from_str(&result.ok_or(Error::IntegrityFailure)?)?,
            seq.to_string(),
        )))
    }

    fn import_response(
        &mut self,
        command_id: &str,
        result: ImportResult,
        seq: String,
        replayed: bool,
    ) -> AttachmentImport {
        self.attachment_previews
            .retain(|_, (_, expiry)| *expiry > Instant::now());
        // Keep grants bounded during large imports; committed revision reads are unaffected.
        if self.attachment_previews.len() >= 512
            && let Some(oldest) = self
                .attachment_previews
                .iter()
                .min_by_key(|(_, (_, expires))| *expires)
                .map(|(token, _)| token.clone())
        {
            self.attachment_previews.remove(&oldest);
        }
        let token = Uuid::new_v4().to_string();
        self.attachment_previews.insert(
            token.clone(),
            (result.blob.hash.clone(), Instant::now() + PREVIEW_TTL),
        );
        AttachmentImport {
            blob: result.blob,
            original_name: result.original_name,
            command_id: command_id.into(),
            change_seq: seq,
            replayed,
            preview_token: token,
            preview_expires_at: (Utc::now()
                + chrono::Duration::seconds(PREVIEW_TTL.as_secs() as i64))
            .to_rfc3339(),
        }
    }

    fn blob_info(&self, hash: &str) -> Result<BlobInfo> {
        validate_hash(hash)?;
        self.connection
            .query_row(
                "SELECT hash,size,media_type,width,height FROM blobs WHERE hash=?1",
                [hash],
                |r| {
                    Ok(BlobInfo {
                        hash: r.get(0)?,
                        size: r.get(1)?,
                        media_type: r.get(2)?,
                        width: r.get(3)?,
                        height: r.get(4)?,
                    })
                },
            )
            .optional()?
            .ok_or(Error::NotFound)
    }

    pub(crate) fn blob_path(&self, hash: &str, create: bool) -> Result<PathBuf> {
        validate_hash(hash)?;
        let workspace = self
            .database_path
            .parent()
            .ok_or(Error::InvalidInput("missing workspace directory"))?;
        let mut directory = workspace.to_owned();
        for name in ["blobs", "sha256", &hash[..2]] {
            directory.push(name);
            if create && !directory.exists() {
                fs::create_dir(&directory).or_else(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })?;
            }
            let metadata = fs::symlink_metadata(&directory)?;
            if !metadata.is_dir()
                || crate::portable::redirected(&metadata)
                || !directory.canonicalize()?.starts_with(workspace)
            {
                return Err(Error::InvalidInput(
                    "managed blob directory cannot redirect outside the workspace",
                ));
            }
        }
        let path = directory.join(hash);
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && (!metadata.is_file() || crate::portable::redirected(&metadata))
        {
            return Err(Error::IntegrityFailure);
        }
        Ok(path)
    }

    pub(crate) fn publish_blob(&self, info: &BlobInfo, bytes: &[u8]) -> Result<()> {
        let path = self.blob_path(&info.hash, true)?;
        if path.exists() {
            self.read_blob(info)?;
            return Ok(());
        }
        let parent = path.parent().ok_or(Error::IntegrityFailure)?;
        let mut temporary = tempfile::Builder::new()
            .prefix("capture-")
            .suffix(".partial")
            .tempfile_in(parent)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.read_blob(info)?;
            }
            Err(error) => return Err(Error::Io(error.error)),
        }
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    fn read_blob(&self, info: &BlobInfo) -> Result<Vec<u8>> {
        if info.size as usize > MAX_IMAGE_BYTES {
            return Err(Error::IntegrityFailure);
        }
        let file = fs::File::open(self.blob_path(&info.hash, false)?)?;
        if file.metadata()?.len() != u64::from(info.size) {
            return Err(Error::IntegrityFailure);
        }
        let mut bytes = Vec::with_capacity(info.size as usize);
        file.take(MAX_IMAGE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() != info.size as usize || hash(&bytes) != info.hash {
            return Err(Error::IntegrityFailure);
        }
        Ok(bytes)
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::InvalidInput("invalid blob hash"));
    }
    Ok(())
}
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.chars().count() > 255
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput(
            "image name must be a filename with 1-255 characters",
        ));
    }
    Ok(())
}
pub(crate) fn decode_image(bytes: &[u8], hash: String) -> Result<BlobInfo> {
    let format =
        image::guess_format(bytes).map_err(|_| Error::InvalidInput("not a supported image"))?;
    let media_type = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::WebP => "image/webp",
        _ => {
            return Err(Error::InvalidInput(
                "only PNG, JPEG and WebP images are supported",
            ));
        }
    };
    let dimensions = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| Error::InvalidInput("invalid image header"))?;
    if dimensions.0 == 0
        || dimensions.1 == 0
        || dimensions.0 > MAX_SIDE
        || dimensions.1 > MAX_SIDE
        || u64::from(dimensions.0) * u64::from(dimensions.1) > MAX_PIXELS
    {
        return Err(Error::InvalidInput(
            "image dimensions exceed supported limits",
        ));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| Error::InvalidInput("image cannot be decoded within resource limits"))?;
    if decoded.dimensions() != dimensions {
        return Err(Error::IntegrityFailure);
    }
    Ok(BlobInfo {
        hash,
        size: bytes.len() as u32,
        media_type: media_type.into(),
        width: dimensions.0,
        height: dimensions.1,
    })
}

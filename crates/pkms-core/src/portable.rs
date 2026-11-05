//! Complete, bounded, immutable portable packages. No app catalog or drafts.
use crate::{
    BACKUP_FORMAT, BACKUP_MAX_BYTES, BACKUP_MAX_DATABASE_BYTES, BACKUP_MAX_ENTRIES,
    BACKUP_MAX_MANIFEST_BYTES, BackupChatCounts, BackupFile, BackupInspection, BackupManifest,
    BackupReport, Error, Library, Result,
    portable_validation::{inspect_database, read_only, validate_database},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const README: &str = "spanseek 完整备份 v1\n请在 spanseek 中校验本文件，再恢复到一个全新目录。原库不会被覆盖。\n包含数据库及所有已提交历史修订引用的图片；包含回收内容。\n不包含未提交草稿、尚未被修订引用的导入图片、本机设置、密钥、模型或日志。\n不要仅复制 library.sqlite3 而遗失 blobs。此包无加密，按个人知识数据保管。\n";

pub(crate) fn archive_error(_: zip::result::ZipError) -> Error {
    Error::InvalidInput("invalid or unsupported ZIP package")
}
pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn stream_copy(
    reader: &mut impl Read,
    writer: &mut impl Write,
    expected_size: u64,
    check: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<String> {
    let mut buf = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    let mut hash = Sha256::new();
    loop {
        check("copy_bytes")?;
        let size = reader.read(&mut buf)?;
        if size == 0 {
            break;
        }
        copied = copied
            .checked_add(size as u64)
            .ok_or(Error::IntegrityFailure)?;
        if copied > expected_size {
            return Err(Error::IntegrityFailure);
        }
        writer.write_all(&buf[..size])?;
        hash.update(&buf[..size]);
    }
    if copied != expected_size {
        return Err(Error::IntegrityFailure);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn new_output(destination: &Path) -> Result<tempfile::NamedTempFile> {
    if fs::symlink_metadata(destination).is_ok() {
        return Err(Error::InvalidInput("output already exists"));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(tempfile::Builder::new()
        .prefix(".pkms-package-")
        .suffix(".partial")
        .tempfile_in(parent)?)
}
pub(crate) fn publish_file(file: tempfile::NamedTempFile, destination: &Path) -> Result<()> {
    file.as_file().sync_all()?;
    file.persist_noclobber(destination)
        .map_err(|e| Error::Io(e.error))?;
    #[cfg(unix)]
    File::open(destination.parent().unwrap_or(Path::new(".")))?.sync_all()?;
    Ok(())
}
pub(crate) fn zip_bytes(writer: &mut ZipWriter<&mut File>, path: &str, bytes: &[u8]) -> Result<()> {
    writer
        .start_file(
            path,
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .unix_permissions(0o600),
        )
        .map_err(archive_error)?;
    writer.write_all(bytes)?;
    Ok(())
}

impl Library {
    pub fn create_backup(
        &self,
        destination: impl AsRef<Path>,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<BackupReport> {
        let ready = self.prepare_backup(destination, check)?;
        check("publish_backup")?;
        ready.publish()
    }
    pub fn prepare_backup(
        &self,
        destination: impl AsRef<Path>,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<PreparedOutput<BackupReport>> {
        let destination = destination.as_ref();
        let mut output = new_output(destination)?;
        let temporary = tempfile::Builder::new()
            .prefix(".pkms-snapshot-")
            .tempdir_in(output.path().parent().ok_or(Error::IntegrityFailure)?)?;
        let database_path = temporary.path().join("library.sqlite3");
        check("snapshot")?;
        crate::snapshot::backup_to(&self.connection, &database_path)?;
        let database = read_only(&database_path)?;
        validate_database(&database, check)?;
        let (workspace_id, counts, blobs) = inspect_database(&database)?;
        let size = fs::metadata(&database_path)?.len();
        if size > BACKUP_MAX_DATABASE_BYTES || blobs.len() + 4 > BACKUP_MAX_ENTRIES {
            return Err(Error::InvalidInput("backup exceeds format limits"));
        }
        let total = blobs
            .iter()
            .try_fold(size, |n, b| n.checked_add(b.size))
            .ok_or(Error::IntegrityFailure)?;
        if total > BACKUP_MAX_BYTES - BACKUP_MAX_MANIFEST_BYTES - 4 * 1024 * 1024 {
            return Err(Error::InvalidInput("backup exceeds 2 GiB limit"));
        }
        let mut writer = ZipWriter::new(output.as_file_mut());
        check("copy_database")?;
        writer
            .start_file(
                "library.sqlite3",
                SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Stored)
                    .unix_permissions(0o600),
            )
            .map_err(archive_error)?;
        let database_hash =
            stream_copy(&mut File::open(&database_path)?, &mut writer, size, check)?;
        for blob in &blobs {
            check("copy_blobs")?;
            crate::portable_validation::validate_blob(
                &database,
                &self.blob_path(&blob.sha256, false)?,
                &blob.sha256,
            )?;
            let mut file = File::open(self.blob_path(&blob.sha256, false)?)?;
            writer
                .start_file(
                    &blob.path,
                    SimpleFileOptions::default()
                        .compression_method(CompressionMethod::Stored)
                        .unix_permissions(0o600),
                )
                .map_err(archive_error)?;
            if stream_copy(&mut file, &mut writer, blob.size, check)? != blob.sha256 {
                return Err(Error::IntegrityFailure);
            }
        }
        let manifest = BackupManifest {
            format_version: BACKUP_FORMAT,
            workspace_id,
            app_version: env!("CARGO_PKG_VERSION").into(),
            database_schema: database.pragma_query_value(None, "user_version", |row| row.get(0))?,
            document_schemas: if database.pragma_query_value::<u32, _>(
                None,
                "user_version",
                |row| row.get(0),
            )? >= 15
            {
                crate::SUPPORTED_DOCUMENT_SCHEMAS.to_vec()
            } else {
                vec![1, 2]
            },
            locator_schemas: vec![1],
            created_at: chrono::Utc::now().to_rfc3339(),
            database: BackupFile {
                path: "library.sqlite3".into(),
                sha256: database_hash,
                size,
            },
            blobs,
            counts,
        };
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        if bytes.len() as u64 > BACKUP_MAX_MANIFEST_BYTES {
            return Err(Error::InvalidInput("manifest too large"));
        }
        zip_bytes(&mut writer, "manifest.json", &bytes)?;
        zip_bytes(&mut writer, "RESTORE.txt", README.as_bytes())?;
        zip_bytes(&mut writer, "COMPLETE", hash_bytes(&bytes).as_bytes())?;
        writer.finish().map_err(archive_error)?;
        if output.as_file().metadata()?.len() > BACKUP_MAX_BYTES {
            return Err(Error::InvalidInput("backup exceeds 2 GiB limit"));
        }
        Ok(PreparedOutput {
            file: output,
            destination: destination.to_owned(),
            report: report(manifest, &database)?,
        })
    }
}

fn report(manifest: BackupManifest, database: &rusqlite::Connection) -> Result<BackupReport> {
    let chat = if manifest.database_schema >= 14 {
        database.query_row(
            "SELECT (SELECT count(*) FROM chat_conversations), (SELECT count(*) FROM chat_messages)",
            [],
            |row| Ok(BackupChatCounts { conversations: row.get(0)?, messages: row.get(1)? }),
        )?
    } else {
        BackupChatCounts {
            conversations: 0,
            messages: 0,
        }
    };
    Ok(BackupReport {
        verified_revisions: manifest.counts.revisions,
        verified_blobs: manifest.blobs.len() as u32,
        manifest,
        chat: Some(chat),
    })
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Read a bounded central directory before ZipArchive allocates its index. This
/// also detects duplicate names (some ZIP APIs collapse them in their map).
fn preflight(file: &mut File) -> Result<BTreeMap<String, u64>> {
    let len = file.metadata()?.len();
    if !(22..=BACKUP_MAX_BYTES).contains(&len) {
        return Err(Error::InvalidInput("archive size exceeds limits"));
    }
    let tail_size = len.min(65_557) as usize;
    file.seek(SeekFrom::End(-(tail_size as i64)))?;
    let mut tail = vec![0; tail_size];
    file.read_exact(&mut tail)?;
    let pos = (0..=tail.len() - 22)
        .rev()
        .find(|&i| {
            tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]
                && i + 22 + usize::from(u16_at(&tail, i + 20)) == tail.len()
        })
        .ok_or(Error::IntegrityFailure)?;
    let entries = usize::from(u16_at(&tail, pos + 10));
    let central_size = u64::from(u32_at(&tail, pos + 12));
    let central_offset = u64::from(u32_at(&tail, pos + 16));
    if u16_at(&tail, pos + 4) != 0
        || u16_at(&tail, pos + 6) != 0
        || usize::from(u16_at(&tail, pos + 8)) != entries
        || !(4..=BACKUP_MAX_ENTRIES).contains(&entries)
        || central_size > 8 * 1024 * 1024
        || central_offset.checked_add(central_size) != Some(len - tail_size as u64 + pos as u64)
    {
        return Err(Error::InvalidInput("unsupported ZIP directory or limits"));
    }
    file.seek(SeekFrom::Start(central_offset))?;
    let mut central = vec![0; central_size as usize];
    file.read_exact(&mut central)?;
    let mut index = 0;
    let mut paths = BTreeMap::new();
    let mut total = 0_u64;
    for _ in 0..entries {
        if index + 46 > central.len() || central[index..index + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err(Error::IntegrityFailure);
        }
        let name_len = usize::from(u16_at(&central, index + 28));
        let end = index
            + 46
            + name_len
            + usize::from(u16_at(&central, index + 30))
            + usize::from(u16_at(&central, index + 32));
        if end > central.len() || name_len > 256 || u16_at(&central, index + 34) != 0 {
            return Err(Error::IntegrityFailure);
        }
        let name = std::str::from_utf8(&central[index + 46..index + 46 + name_len])
            .map_err(|_| Error::InvalidInput("invalid archive path"))?;
        let size = u64::from(u32_at(&central, index + 24));
        let mode = u32_at(&central, index + 38) >> 16;
        let dos = u32_at(&central, index + 38) & 0xffff;
        if !allowed_path(name)
            || (mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
            || dos & 0x410 != 0
            || u16_at(&central, index + 10) != 0
            || u16_at(&central, index + 8) & (1 | 64) != 0
            || u64::from(u32_at(&central, index + 20)) != size
        {
            return Err(Error::InvalidInput(
                "unsupported archive entry, link, or path",
            ));
        }
        let max = match name {
            "manifest.json" => BACKUP_MAX_MANIFEST_BYTES,
            "library.sqlite3" => BACKUP_MAX_DATABASE_BYTES,
            "RESTORE.txt" => 16 * 1024,
            "COMPLETE" => 64,
            _ => 20 * 1024 * 1024,
        };
        total = total.checked_add(size).ok_or(Error::IntegrityFailure)?;
        if size == 0
            || size > max
            || total > BACKUP_MAX_BYTES
            || paths.insert(name.to_owned(), size).is_some()
        {
            return Err(Error::InvalidInput(
                "duplicate entry or package limit exceeded",
            ));
        }
        index = end;
    }
    if index != central.len() {
        return Err(Error::IntegrityFailure);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(paths)
}

fn allowed_path(path: &str) -> bool {
    if matches!(
        path,
        "manifest.json" | "library.sqlite3" | "RESTORE.txt" | "COMPLETE"
    ) {
        return true;
    }
    let parts: Vec<_> = path.split('/').collect();
    parts.len() == 4
        && parts[0] == "blobs"
        && parts[1] == "sha256"
        && valid_hash(parts[3])
        && parts[2] == &parts[3][..2]
}

struct Package {
    archive: ZipArchive<File>,
    manifest: BackupManifest,
}
fn package(path: &Path) -> Result<Package> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || redirected(&metadata) {
        return Err(Error::InvalidInput("backup input must be a regular file"));
    }
    let mut file = File::open(path)?;
    let paths = preflight(&mut file)?;
    let mut archive = ZipArchive::new(file).map_err(archive_error)?;
    if archive.len() != paths.len() {
        return Err(Error::IntegrityFailure);
    }
    let mut manifest_bytes = Vec::new();
    archive
        .by_name("manifest.json")
        .map_err(archive_error)?
        .take(BACKUP_MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut manifest_bytes)?;
    if manifest_bytes.len() as u64 > BACKUP_MAX_MANIFEST_BYTES {
        return Err(Error::InvalidInput("manifest too large"));
    }
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.format_version != BACKUP_FORMAT
        || !(7..=crate::migrations::CURRENT_SCHEMA_VERSION as u32)
            .contains(&manifest.database_schema)
        || manifest.document_schemas
            != if manifest.database_schema >= 15 {
                vec![1, 2, 3]
            } else {
                vec![1, 2]
            }
        || manifest.locator_schemas
            != if manifest.database_schema >= 9 {
                vec![1]
            } else {
                vec![]
            }
    {
        return Err(Error::UnsupportedDatabase);
    }
    crate::document::valid_uuid(&manifest.workspace_id)?;
    if manifest.app_version.is_empty()
        || manifest.app_version.len() > 64
        || chrono::DateTime::parse_from_rfc3339(&manifest.created_at).is_err()
    {
        return Err(Error::InvalidInput("invalid backup metadata"));
    }
    let mut complete = String::new();
    archive
        .by_name("COMPLETE")
        .map_err(archive_error)?
        .take(65)
        .read_to_string(&mut complete)?;
    if complete != hash_bytes(&manifest_bytes) {
        return Err(Error::IntegrityFailure);
    }
    let mut instructions = Vec::new();
    archive
        .by_name("RESTORE.txt")
        .map_err(archive_error)?
        .take(16 * 1024 + 1)
        .read_to_end(&mut instructions)?;
    if instructions.is_empty() || instructions.len() > 16 * 1024 {
        return Err(Error::IntegrityFailure);
    }
    let mut expected = BTreeSet::from([
        "manifest.json".to_owned(),
        "RESTORE.txt".to_owned(),
        "COMPLETE".to_owned(),
    ]);
    if manifest.database.path != "library.sqlite3" {
        return Err(Error::InvalidInput("invalid database path"));
    }
    for entry in std::iter::once(&manifest.database).chain(&manifest.blobs) {
        if !allowed_path(&entry.path)
            || !valid_hash(&entry.sha256)
            || paths.get(&entry.path) != Some(&entry.size)
            || !expected.insert(entry.path.clone())
        {
            return Err(Error::IntegrityFailure);
        }
        if entry.path != "library.sqlite3"
            && entry.path != format!("blobs/sha256/{}/{}", &entry.sha256[..2], entry.sha256)
        {
            return Err(Error::IntegrityFailure);
        }
    }
    if expected != paths.into_keys().collect() {
        return Err(Error::IntegrityFailure);
    }
    Ok(Package { archive, manifest })
}

pub fn inspect_backup(path: impl AsRef<Path>) -> Result<BackupInspection> {
    Ok(BackupInspection {
        manifest: package(path.as_ref())?.manifest,
        fully_verified: false,
    })
}

fn extract(
    mut package: Package,
    directory: &Path,
    check: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<BackupReport> {
    for entry in std::iter::once(&package.manifest.database).chain(&package.manifest.blobs) {
        check("verify_files")?;
        let path = directory.join(&entry.path);
        fs::create_dir_all(path.parent().ok_or(Error::IntegrityFailure)?)?;
        let mut output = File::create_new(&path)?;
        let mut input = package
            .archive
            .by_name(&entry.path)
            .map_err(archive_error)?;
        if stream_copy(&mut input, &mut output, entry.size, check)? != entry.sha256 {
            return Err(Error::IntegrityFailure);
        }
        output.sync_all()?;
    }
    let db = read_only(&directory.join("library.sqlite3"))?;
    let schema: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if schema != package.manifest.database_schema {
        return Err(Error::UnsupportedDatabase);
    }
    validate_database(&db, check)?;
    for blob in &package.manifest.blobs {
        check("verify_blobs")?;
        crate::portable_validation::validate_blob(&db, &directory.join(&blob.path), &blob.sha256)?;
    }
    let (workspace_id, counts, blobs) = inspect_database(&db)?;
    if workspace_id != package.manifest.workspace_id
        || counts != package.manifest.counts
        || blobs != package.manifest.blobs
    {
        return Err(Error::IntegrityFailure);
    }
    report(package.manifest, &db)
}

pub fn verify_backup(
    path: impl AsRef<Path>,
    check: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<BackupReport> {
    let package = package(path.as_ref())?;
    let temp = tempfile::Builder::new().prefix("pkms-verify-").tempdir()?;
    extract(package, temp.path(), check)
}

pub struct PreparedRestore {
    staging: tempfile::TempDir,
    destination: PathBuf,
    pub report: BackupReport,
}

pub fn prepare_restore(
    path: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    check: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<PreparedRestore> {
    let destination = destination.as_ref();
    if fs::symlink_metadata(destination).is_ok() {
        return Err(Error::InvalidInput("restore destination already exists"));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let destination = parent.join(
        destination
            .file_name()
            .ok_or(Error::InvalidInput("missing destination name"))?,
    );
    let package = package(path.as_ref())?;
    let staging = tempfile::Builder::new()
        .prefix(".pkms-restore-")
        .tempdir_in(&parent)?;
    let report = extract(package, staging.path(), check)?;
    // The package has already passed all read-only checks before a writable open.
    // Rebuild FTS from current documents; it is derived data, not backup authority.
    {
        let mut library = Library::open_existing(staging.path())?;
        let tx = library.connection.transaction()?;
        tx.execute("DELETE FROM active_index_generations", [])?;
        tx.execute("INSERT INTO index_dirty_objects(object_id) SELECT id FROM objects WHERE kind IN ('note','source') ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL", [])?;
        tx.execute("UPDATE index_rebuild_jobs SET status='failed',error_code='interrupted',lease_token=NULL,lease_expires_at=NULL,updated_at=?1 WHERE status IN ('queued','running')",[chrono::Utc::now().to_rfc3339()])?;
        tx.execute("UPDATE index_generations SET status='failed',error_code='projection_rebuild_required' WHERE status IN ('ready','running')", [])?;
        tx.execute(
            "UPDATE index_generations SET lease_token=NULL,lease_expires_at=NULL",
            [],
        )?;
        tx.execute("INSERT INTO answer_events(answer_id,seq,status,text_delta) SELECT a.id,coalesce((SELECT max(e.seq) FROM answer_events e WHERE e.answer_id=a.id),0)+1,'failed',NULL FROM answers a WHERE a.status IN ('queued','running')", [])?;
        tx.execute("UPDATE answers SET status='failed',error_code='interrupted',updated_at=?1 WHERE status IN ('queued','running')", [chrono::Utc::now().to_rfc3339()])?;
        tx.execute(
            "UPDATE answers SET lease_token=NULL,lease_expires_at=NULL",
            [],
        )?;
        crate::chat::interrupt_restored_runs(&tx)?;
        tx.execute("UPDATE portable_jobs SET status='failed',stage='restored_copy',lease_id=NULL,error_code='restored_copy' WHERE status IN ('queued','running')",[])?;
        tx.execute("UPDATE source_jobs SET status=CASE WHEN source_version_id IS NULL OR attempt>=3 THEN 'failed' ELSE 'queued' END,stage=CASE WHEN source_version_id IS NULL THEN 'input_unavailable' WHEN attempt>=3 THEN 'attempts_exhausted' ELSE 'resume_pending' END,lease_token=NULL,lease_expires_at=NULL,error_code=CASE WHEN source_version_id IS NULL THEN 'input_unavailable' WHEN attempt>=3 THEN 'attempts_exhausted' ELSE NULL END WHERE status IN ('queued','running')",[])?;
        tx.execute("UPDATE source_job_attempts SET finished_at=coalesce(finished_at,?1),error_code=coalesce(error_code,'restored_copy') WHERE finished_at IS NULL",[chrono::Utc::now().to_rfc3339()])?;
        tx.execute("DELETE FROM note_fts", [])?;
        let ids = tx
            .prepare("SELECT id FROM notes")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for id in ids {
            check("rebuild_search")?;
            let note = crate::notes::read_note(&tx, &id)?;
            crate::insert_note_index(
                &tx,
                &id,
                &crate::tokenize(&note.title),
                &crate::tokenize(&note.document.plain_text()),
            )?;
        }
        tx.commit()?;
        library.rebuild_source_search(check)?;
        library
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    }
    File::options()
        .write(true)
        .open(staging.path().join("library.sqlite3"))?
        .sync_all()?;
    check("restore_ready")?;
    Ok(PreparedRestore {
        staging,
        destination,
        report,
    })
}

impl PreparedRestore {
    pub fn staging_path(&self) -> &Path {
        self.staging.path()
    }
    /// The application may write an operation marker before this non-replacing
    /// publication, then atomically register its preallocated instance ID.
    pub fn publish(self) -> Result<BackupReport> {
        #[cfg(windows)]
        rename_directory_noclobber(self.staging.path(), &self.destination)?;
        #[cfg(target_os = "linux")]
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            self.staging.path(),
            rustix::fs::CWD,
            &self.destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)?;
        #[cfg(not(any(windows, target_os = "linux")))]
        return Err(Error::InvalidInput(
            "atomic restore publication is not supported on this platform",
        ));
        #[cfg(unix)]
        File::open(self.destination.parent().ok_or(Error::IntegrityFailure)?)?.sync_all()?;
        Ok(self.report)
    }
}

pub(crate) fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub struct PreparedOutput<T> {
    pub(crate) file: tempfile::NamedTempFile,
    pub(crate) destination: PathBuf,
    pub report: T,
}
impl<T> PreparedOutput<T> {
    pub fn artifact_path(&self) -> &Path {
        self.file.path()
    }
    pub fn publish(self) -> Result<T> {
        publish_file(self.file, &self.destination)?;
        Ok(self.report)
    }
    pub fn map_report<U>(self, map: impl FnOnce(T) -> U) -> PreparedOutput<U> {
        PreparedOutput {
            file: self.file,
            destination: self.destination,
            report: map(self.report),
        }
    }
}

#[cfg(windows)]
fn rename_directory_noclobber(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let wide = |path: &Path| -> std::io::Result<Vec<u16>> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        value.push(0);
        Ok(value)
    };
    let from = wide(from)?;
    let to = wide(to)?;
    // SAFETY: Both UTF-16 vectors are NUL-terminated and remain live throughout
    // the call. Omitting REPLACE_EXISTING is essential even for empty targets.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

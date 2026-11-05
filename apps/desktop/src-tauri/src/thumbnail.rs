use crate::runtime::lock;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, metadata::Orientation};
use pkms_app::{ApiError, ApiErrorCode, WorkspaceIdentity};
use pkms_core::{BlobInfo, Error};
use std::{
    collections::VecDeque,
    io::Cursor,
    sync::{Arc, Condvar, Mutex, OnceLock},
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
// Match the limits enforced when the immutable original is imported by core.
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_SIDE: u32 = 16_384;
const MAX_PIXELS: u64 = 40_000_000;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const CONCURRENT_READS: usize = 2;
const CACHE_ENTRIES: usize = 64;
const CACHE_BYTES: usize = 16 * 1024 * 1024;

type Resource = (BlobInfo, Vec<u8>);

/// Called on the protocol's blocking worker, never on the UI thread. The loader
/// must authorize the active identity and fixed revision and verify the original
/// hash on every request, including a cache hit. Only PNG thumbnails are cached.
pub fn read(
    identity: &WorkspaceIdentity,
    revision: &str,
    hash: &str,
    load_authorized: impl FnOnce() -> Result<Resource, ApiError>,
) -> Result<Vec<u8>, ApiError> {
    static SERVICE: OnceLock<ThumbnailService> = OnceLock::new();
    SERVICE.get_or_init(ThumbnailService::default).render(
        Key {
            identity: identity.clone(),
            revision: revision.into(),
            hash: hash.into(),
        },
        load_authorized,
    )
}

#[derive(Clone, PartialEq, Eq)]
struct Key {
    identity: WorkspaceIdentity,
    revision: String,
    hash: String,
}

struct ThumbnailService {
    gate: Gate,
    cache: Mutex<Cache>,
}

impl Default for ThumbnailService {
    fn default() -> Self {
        Self {
            gate: Gate::default(),
            cache: Mutex::new(Cache::new(CACHE_ENTRIES, CACHE_BYTES)),
        }
    }
}

impl ThumbnailService {
    fn render(
        &self,
        key: Key,
        load_authorized: impl FnOnce() -> Result<Resource, ApiError>,
    ) -> Result<Vec<u8>, ApiError> {
        // Acquire before loading: queued requests must not each hold an original
        // of up to 20 MiB. Neither waiting nor decoding holds the AppService lock.
        let _permit = self.gate.acquire()?;
        let (info, bytes) = load_authorized()?;
        let format = validate_input(&key, &info, &bytes)
            .map_err(|error| ApiError::from_app(error.into(), ""))?;
        let cached = lock(&self.cache, "")?.get(&key);
        if let Some(cached) = cached {
            return Ok(cached.as_ref().clone());
        }
        let rendered = Arc::new(
            render_png(&info, &bytes, format)
                .map_err(|error| ApiError::from_app(error.into(), ""))?,
        );
        drop(bytes);
        lock(&self.cache, "")?.insert(key, rendered.clone());
        Ok(rendered.as_ref().clone())
    }
}

#[derive(Default)]
struct Gate {
    active: Mutex<usize>,
    available: Condvar,
}

impl Gate {
    fn acquire(&self) -> Result<Permit<'_>, ApiError> {
        let mut active = lock(&self.active, "")?;
        while *active >= CONCURRENT_READS {
            active = self
                .available
                .wait(active)
                .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?;
        }
        *active += 1;
        Ok(Permit(self))
    }
}

struct Permit<'a>(&'a Gate);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut active = self.0.active.lock().unwrap_or_else(|p| p.into_inner());
        *active -= 1;
        self.0.available.notify_one();
    }
}

struct Cache {
    entries: VecDeque<(Key, Arc<Vec<u8>>)>,
    bytes: usize,
    entry_limit: usize,
    byte_limit: usize,
}

impl Cache {
    fn new(entry_limit: usize, byte_limit: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            entry_limit,
            byte_limit,
        }
    }

    fn get(&mut self, key: &Key) -> Option<Arc<Vec<u8>>> {
        let index = self.entries.iter().position(|(stored, _)| stored == key)?;
        let entry = self.entries.remove(index)?;
        let result = entry.1.clone();
        self.entries.push_back(entry);
        Some(result)
    }

    fn insert(&mut self, key: Key, png: Arc<Vec<u8>>) {
        if let Some(index) = self.entries.iter().position(|(stored, _)| stored == &key)
            && let Some((_, prior)) = self.entries.remove(index)
        {
            self.bytes -= prior.len();
        }
        if png.len() > self.byte_limit || self.entry_limit == 0 {
            return;
        }
        while self.entries.len() >= self.entry_limit || self.bytes + png.len() > self.byte_limit {
            if let Some((_, prior)) = self.entries.pop_front() {
                self.bytes -= prior.len();
            }
        }
        self.bytes += png.len();
        self.entries.push_back((key, png));
    }
}

fn validate_input(key: &Key, info: &BlobInfo, bytes: &[u8]) -> pkms_core::Result<ImageFormat> {
    if bytes.is_empty()
        || bytes.len() > MAX_IMAGE_BYTES
        || bytes.len() != info.size as usize
        || info.hash != key.hash
    {
        return Err(Error::IntegrityFailure);
    }
    validate_dimensions(info.width, info.height)?;
    let format = image::guess_format(bytes)
        .map_err(|_| Error::InvalidInput("invalid thumbnail image format"))?;
    let mime = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::WebP => "image/webp",
        _ => return Err(Error::InvalidInput("unsupported thumbnail image format")),
    };
    if mime != info.media_type {
        return Err(Error::IntegrityFailure);
    }
    Ok(format)
}

fn validate_dimensions(width: u32, height: u32) -> pkms_core::Result<()> {
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(Error::InvalidInput(
            "image dimensions exceed thumbnail limits",
        ));
    }
    Ok(())
}

fn render_png(info: &BlobInfo, bytes: &[u8], format: ImageFormat) -> pkms_core::Result<Vec<u8>> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits.clone());
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| Error::InvalidInput("image cannot be decoded within thumbnail limits"))?;
    let dimensions = decoder.dimensions();
    validate_dimensions(dimensions.0, dimensions.1)?;
    if dimensions != (info.width, info.height) {
        return Err(Error::IntegrityFailure);
    }
    // into_decoder does not reserve the output buffer as ImageReader::decode
    // does. Preserve that check before DynamicImage allocates any pixels.
    limits
        .reserve(decoder.total_bytes())
        .map_err(|_| Error::InvalidInput("image exceeds thumbnail allocation budget"))?;
    decoder
        .set_limits(limits)
        .map_err(|_| Error::InvalidInput("image cannot be decoded within thumbnail limits"))?;
    let orientation = decoder
        .orientation()
        .map_err(|_| Error::InvalidInput("invalid thumbnail image orientation"))?;
    let decoded = DynamicImage::from_decoder(decoder)
        .map_err(|_| Error::InvalidInput("invalid thumbnail image data"))?;
    let (width, height) = match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (HEIGHT, WIDTH),
        _ => (WIDTH, HEIGHT),
    };
    let mut small = if decoded.width() > width || decoded.height() > height {
        let small = decoded.thumbnail(width, height);
        drop(decoded);
        small
    } else {
        decoded
    };
    // Rotate only the small image; a portrait original need not be duplicated.
    small.apply_orientation(orientation);
    if small.width() > WIDTH || small.height() > HEIGHT {
        return Err(Error::IntegrityFailure);
    }
    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(small.into_rgba8())
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| Error::InvalidInput("thumbnail cannot be encoded"))?;
    Ok(output.into_inner())
}

#[cfg(test)]
#[path = "thumbnail_tests.rs"]
mod tests;

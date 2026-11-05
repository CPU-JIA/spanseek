use super::*;
use image::{GenericImageView, Rgb, RgbImage, Rgba, RgbaImage};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{atomic::AtomicUsize, atomic::Ordering, mpsc},
    time::Duration,
};

fn key() -> Key {
    Key {
        identity: WorkspaceIdentity {
            local_instance_id: uuid::Uuid::new_v4().to_string(),
            workspace_id: uuid::Uuid::new_v4().to_string(),
        },
        revision: uuid::Uuid::new_v4().to_string(),
        hash: "a".repeat(64),
    }
}

fn resource(width: u32, height: u32, format: ImageFormat) -> Resource {
    let image = if format == ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb([64, 128, 192])))
    } else {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            width,
            height,
            Rgba([64, 128, 192, 96]),
        ))
    };
    encoded(image, format)
}

fn encoded(image: DynamicImage, format: ImageFormat) -> Resource {
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, format).unwrap();
    let bytes = output.into_inner();
    (
        BlobInfo {
            hash: "a".repeat(64),
            size: bytes.len() as u32,
            width: image.width(),
            height: image.height(),
            media_type: format.to_mime_type().into(),
        },
        bytes,
    )
}

fn png(resource: &Resource) -> pkms_core::Result<Vec<u8>> {
    let format = validate_input(&key(), &resource.0, &resource.1)?;
    render_png(&resource.0, &resource.1, format)
}

#[test]
fn supported_formats_produce_bounded_pngs_without_upscaling() {
    for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        for ((width, height), expected) in [
            ((960, 640), (480, 320)),
            ((640, 960), (213, 320)),
            ((1200, 120), (480, 48)),
            ((120, 1200), (32, 320)),
            ((8, 6), (8, 6)),
        ] {
            let bytes = png(&resource(width, height, format)).unwrap();
            assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Png);
            assert_eq!(
                image::load_from_memory(&bytes).unwrap().dimensions(),
                expected
            );
            assert!(bytes.len() < 1024 * 1024);
        }
    }
}

#[test]
fn transparent_pixels_survive_png_and_webp_thumbnails() {
    for format in [ImageFormat::Png, ImageFormat::WebP] {
        for (width, height) in [(8, 6), (960, 640)] {
            let result = image::load_from_memory(&png(&resource(width, height, format)).unwrap())
                .unwrap()
                .into_rgba8();
            assert_eq!(*result.get_pixel(0, 0), Rgba([64, 128, 192, 96]));
        }
    }
}

fn with_orientation(mut resource: Resource, orientation: u8) -> Resource {
    // A minimal little-endian TIFF IFD embedded in a real JPEG APP1 segment.
    let mut exif =
        b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\0\0\0\0\0\0\0\0".to_vec();
    exif[24] = orientation;
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1];
    bytes.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    bytes.extend_from_slice(&exif);
    bytes.extend_from_slice(&resource.1[2..]);
    resource.0.size = bytes.len() as u32;
    resource.1 = bytes;
    resource
}

#[test]
fn exif_orientation_is_applied_once_and_uses_the_oriented_bounds() {
    let source = encoded(
        DynamicImage::ImageRgb8(RgbImage::from_fn(80, 40, |x, _| {
            if x < 40 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        })),
        ImageFormat::Jpeg,
    );
    let rotated = image::load_from_memory(&png(&with_orientation(source.clone(), 6)).unwrap())
        .unwrap()
        .into_rgb8();
    assert_eq!(rotated.dimensions(), (40, 80));
    assert!(rotated.get_pixel(20, 10)[0] > 200);
    assert!(rotated.get_pixel(20, 70)[2] > 200);
    let flipped = image::load_from_memory(&png(&with_orientation(source, 2)).unwrap())
        .unwrap()
        .into_rgb8();
    assert_eq!(flipped.dimensions(), (80, 40));
    assert!(flipped.get_pixel(10, 20)[2] > 200);
    assert!(flipped.get_pixel(70, 20)[0] > 200);
    let large = png(&with_orientation(resource(960, 640, ImageFormat::Jpeg), 6)).unwrap();
    assert_eq!(
        image::load_from_memory(&large).unwrap().dimensions(),
        (213, 320)
    );
}

#[test]
fn malformed_unsupported_and_mismatched_resources_are_rejected() {
    let original = resource(8, 6, ImageFormat::Png);
    let mut changed = original.clone();
    changed.0.hash = "b".repeat(64);
    assert!(png(&changed).is_err());
    changed = original.clone();
    changed.0.size += 1;
    assert!(png(&changed).is_err());
    changed = original.clone();
    changed.0.width += 1;
    assert!(matches!(png(&changed), Err(Error::IntegrityFailure)));
    changed = original.clone();
    changed.0.media_type = "image/jpeg".into();
    assert!(matches!(png(&changed), Err(Error::IntegrityFailure)));
    for bytes in [
        Vec::new(),
        b"not an image".to_vec(),
        b"GIF89a\x01\0\x01\0\0\0\0".to_vec(),
        original.1[..original.1.len() / 2].to_vec(),
        vec![0; MAX_IMAGE_BYTES + 1],
    ] {
        changed = original.clone();
        changed.0.size = bytes.len() as u32;
        changed.1 = bytes;
        assert!(png(&changed).is_err());
    }
    for (width, height) in [
        (0, 6),
        (8, 0),
        (MAX_SIDE + 1, 1),
        (1, MAX_SIDE + 1),
        (8000, 5001),
    ] {
        changed = original.clone();
        changed.0.width = width;
        changed.0.height = height;
        assert!(png(&changed).is_err());
    }
}

#[test]
fn sixteen_bit_declared_output_cannot_bypass_the_decode_budget() {
    let mut source = encoded(DynamicImage::new_rgba16(1, 1), ImageFormat::Png);
    source.0.width = 8000;
    source.0.height = 5000;
    source.1[16..20].copy_from_slice(&8000_u32.to_be_bytes());
    source.1[20..24].copy_from_slice(&5000_u32.to_be_bytes());
    // Update the IHDR CRC. The deliberately tiny IDAT must never be decoded:
    // a 40-million-pixel RGBA16 output already exceeds the allocation budget.
    let mut crc = !0_u32;
    for byte in &source.1[12..29] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    source.1[29..33].copy_from_slice(&(!crc).to_be_bytes());
    assert!(matches!(
        png(&source),
        Err(Error::InvalidInput(
            "image exceeds thumbnail allocation budget"
        ))
    ));
}

#[test]
fn cache_hits_still_load_and_authorize_the_original() {
    let service = ThumbnailService::default();
    let key = key();
    let original = resource(8, 6, ImageFormat::Png);
    let loads = AtomicUsize::new(0);
    let expected = service
        .render(key.clone(), || {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok(original.clone())
        })
        .unwrap();
    let repeated = service
        .render(key.clone(), || {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok(original.clone())
        })
        .unwrap();
    assert_eq!(expected, repeated);
    assert_eq!(service.cache.lock().unwrap().entries.len(), 1);
    assert!(
        service
            .render(key, || {
                loads.fetch_add(1, Ordering::SeqCst);
                Err(ApiError::new(ApiErrorCode::InvalidInput, ""))
            })
            .is_err()
    );
    assert_eq!(loads.load(Ordering::SeqCst), 3);
    assert_eq!(*service.gate.active.lock().unwrap(), 0);
}

#[test]
fn cache_is_lru_bounded_by_both_count_and_bytes_and_uses_both_identities() {
    let first = key();
    let mut by_instance = first.clone();
    by_instance.identity.local_instance_id = uuid::Uuid::new_v4().to_string();
    let mut by_workspace = first.clone();
    by_workspace.identity.workspace_id = uuid::Uuid::new_v4().to_string();
    let mut by_revision = first.clone();
    by_revision.revision = uuid::Uuid::new_v4().to_string();
    let mut by_hash = first.clone();
    by_hash.hash = "b".repeat(64);
    let mut cache = Cache::new(4, 8);
    for key in [&first, &by_instance, &by_workspace, &by_revision] {
        cache.insert(key.clone(), Arc::new(vec![1, 2]));
    }
    assert_eq!(cache.entries.len(), 4);
    assert_eq!(cache.bytes, 8);
    assert!(cache.get(&first).is_some());
    cache.insert(by_hash.clone(), Arc::new(vec![3, 4]));
    assert!(cache.get(&by_instance).is_none());
    assert!(cache.get(&first).is_some());
    cache.insert(by_hash.clone(), Arc::new(vec![3; 7]));
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(cache.bytes, 7);
    assert!(cache.get(&by_hash).is_some());
    cache.insert(first.clone(), Arc::new(vec![0; 9]));
    assert!(cache.get(&first).is_none());
    assert_eq!(cache.bytes, 7);
    let mut count_limited = Cache::new(1, 8);
    count_limited.insert(first.clone(), Arc::new(vec![1]));
    count_limited.insert(by_hash, Arc::new(vec![2]));
    assert_eq!(count_limited.entries.len(), 1);
    assert!(count_limited.get(&first).is_none());
}

#[test]
fn queued_work_waits_before_loading_and_permits_release_on_failure_and_unwind() {
    let service = ThumbnailService::default();
    let first = service.gate.acquire().unwrap();
    let second = service.gate.acquire().unwrap();
    let (loaded, received) = mpsc::channel();
    std::thread::scope(|scope| {
        let task = scope.spawn(|| {
            service.render(key(), || {
                loaded.send(()).unwrap();
                Ok(resource(8, 6, ImageFormat::Png))
            })
        });
        assert!(received.recv_timeout(Duration::from_millis(80)).is_err());
        drop(first);
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        task.join().unwrap().unwrap();
    });
    assert_eq!(*service.gate.active.lock().unwrap(), 1);
    drop(second);
    assert!(
        service
            .render(key(), || Err(ApiError::new(ApiErrorCode::Internal, "")))
            .is_err()
    );
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let _ = service.render(key(), || panic!("controlled loader failure"));
        }))
        .is_err()
    );
    assert_eq!(*service.gate.active.lock().unwrap(), 0);
    service
        .render(key(), || Ok(resource(8, 6, ImageFormat::Png)))
        .unwrap();
}

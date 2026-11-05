use pkms_core::*;
use std::fs;
use tempfile::tempdir;

fn write_entry<W: std::io::Write>(builder: &mut tar::Builder<W>, path: &str, bytes: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, path, bytes).unwrap();
}

#[test]
fn jex_import_converts_headers_resources_and_skips_deleted_notes() {
    let dir = tempdir().unwrap();
    let jex = dir.path().join("export.jex");
    let image = {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    };
    {
        let file = fs::File::create(&jex).unwrap();
        let mut builder = tar::Builder::new(file);
        write_entry(&mut builder, "manifest.json", br#"{"version":1}"#);
        write_entry(
            &mut builder,
            "aaaa1111.md",
            "title: 会议纪要\nupdated_time: 2024-05-01T10:00:00Z\ntype_: 1\ndeleted_time: 0\n\n讨论了两件事。\n\n![图](:/res1.png)\n".as_bytes(),
        );
        write_entry(
            &mut builder,
            "bbbb2222.md",
            "title: 已删除笔记\ntype_: 1\ndeleted_time: 1700000000000\n\n不该出现的内容\n"
                .as_bytes(),
        );
        write_entry(&mut builder, "resources/res1.png", &image);
        builder.finish().unwrap();
    }

    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let report = library.import_joplin_jex(&jex).unwrap();
    assert_eq!(report.notes, 1, "{report:?}");
    assert_eq!(report.skipped_deleted, 1, "{report:?}");
    assert_eq!(report.resources, 1, "{report:?}");
    assert_eq!(report.vault.notes, 1, "{report:?}");
    assert_eq!(report.vault.attachments, 1, "{report:?}");
    assert_eq!(report.vault.unresolved_links, 0, "{report:?}");
    assert_eq!(report.vault.losses.len(), 2, "{report:?}");

    let notes = library.list_notes().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title, "会议纪要");
    assert!(notes[0].document.plain_text().contains("讨论了两件事"));
    assert!(
        notes[0]
            .document
            .content
            .iter()
            .any(|node| { node.kind == NodeType::Image && node.attrs.blob_hash.is_some() })
    );
}

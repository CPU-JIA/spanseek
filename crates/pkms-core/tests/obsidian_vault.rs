use pkms_core::*;
use std::fs;
use tempfile::tempdir;

fn sample_vault() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let notes = dir.path().join("notes");
    let attach = dir.path().join("attach");
    fs::create_dir_all(&notes).unwrap();
    fs::create_dir_all(&attach).unwrap();
    fs::write(
        notes.join("方法.md"),
        "---\ntitle: \"方法\"\ntags: [研究, \"写作\"]\n---\n# 方法\n\n反向链接到 [[证据链|证据]]。\n\n- 保留列表\n- 保留 `code`\n",
    )
    .unwrap();
    fs::write(
        notes.join("证据链.md"),
        "---\ntitle: \"证据链\"\ntags:\n  - 研究\n  - 方法\naliases: [证据, 材料]\n---\n# 证据链\n\n这条笔记引用 [[方法]]，并嵌入附件：\n\n![[图.png]]\n\n正文保留原始 Markdown 语法与中文。\n",
    )
    .unwrap();
    fs::write(
        notes.join("缺图.md"),
        "---\ntitle: \"缺图\"\n---\n# 缺图\n\n![[不存在.png]]\n",
    )
    .unwrap();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 30, 30, 255]))
        .save(attach.join("图.png"))
        .unwrap();
    dir
}

#[test]
fn obsidian_vault_round_trip_preserves_links_tags_yaml_and_attachments() {
    let source = sample_vault();
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let report = library
        .import_obsidian_vault(source.path(), "test")
        .unwrap();
    assert_eq!(report.notes, 3, "{report:?}");
    assert_eq!(report.attachments, 1, "{report:?}");
    assert_eq!(report.links, 2, "{report:?}");
    assert_eq!(report.tags, 4, "{report:?}");
    assert_eq!(report.unresolved_links, 0, "{report:?}");
    assert_eq!(report.normalized.len(), 3, "{report:?}");
    assert!(
        report
            .losses
            .iter()
            .any(|loss| loss.contains("未找到附件：不存在.png")),
        "missing attachment must be reported: {report:?}"
    );

    let out = tempdir().unwrap();
    let destination = out.path().join("vault");
    let export = library.export_obsidian_vault(&destination).unwrap();
    assert_eq!(export.notes, 3, "{export:?}");
    assert_eq!(export.attachments, 1, "{export:?}");

    let body = fs::read_to_string(destination.join("notes/证据链.md")).unwrap();
    assert_eq!(
        body.matches("---\n").count(),
        2,
        "exactly one frontmatter block: {body}"
    );
    assert_eq!(
        body.matches("# 证据链").count(),
        1,
        "the heading must not be duplicated: {body}"
    );
    assert!(body.contains("aliases: [证据, 材料]"), "{body}");
    assert!(body.contains("[[方法]]"), "{body}");
    assert!(body.contains("![](../attachments/"), "{body}");
    assert!(!body.contains("&#10;"), "body must not be escaped: {body}");

    let other = fs::read_to_string(destination.join("notes/方法.md")).unwrap();
    assert!(other.contains("[[证据链|证据]]"), "{other}");
    assert!(other.contains("- 保留列表"), "{other}");
    assert!(other.contains("`code`"), "{other}");

    // A second import of the exported vault must reproduce the same body.
    let root2 = tempdir().unwrap();
    let mut library2 = Library::open(root2.path()).unwrap();
    library2
        .import_obsidian_vault(&destination, "test")
        .unwrap();
    let out2 = tempdir().unwrap();
    let destination2 = out2.path().join("vault");
    library2.export_obsidian_vault(&destination2).unwrap();
    let strip = |text: &str| {
        text.lines()
            .filter(|line| !line.starts_with("spanseek_id:") && !line.starts_with("tags:"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    for name in ["方法", "证据链", "缺图"] {
        let first = fs::read_to_string(destination.join(format!("notes/{name}.md"))).unwrap();
        let second = fs::read_to_string(destination2.join(format!("notes/{name}.md"))).unwrap();
        assert_eq!(strip(&first), strip(&second), "round trip drifted: {name}");
    }
}

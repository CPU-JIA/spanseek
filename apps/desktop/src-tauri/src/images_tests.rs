use super::*;
use image::{DynamicImage, GenericImageView, ImageFormat};
use pkms_app::{AppService, GrantPurpose, OperationKind, OperationStatus};
use pkms_core::{CreateNote, Note, NoteDocument, SaveNote};
use serde_json::json;
use std::{fs, io::Cursor, path::Path, path::PathBuf};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

struct TestDirectory {
    path: PathBuf,
    parent: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let path = parent.join(format!("pkms-thumbnail-tests-{}", id()));
        fs::create_dir(&path).unwrap();
        let path = path.canonicalize().unwrap();
        assert_eq!(path.parent(), Some(parent.as_path()));
        Self { path, parent }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        // Remove only this test's freshly created, canonical, direct child.
        let resolved = self.path.canonicalize().unwrap();
        assert_eq!(resolved, self.path);
        assert_eq!(resolved.parent(), Some(self.parent.as_path()));
        assert!(
            resolved
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("pkms-thumbnail-tests-")
        );
        fs::remove_dir_all(resolved).unwrap();
    }
}

fn workspace(service: &mut AppService, path: &Path, create: bool) -> WorkspaceIdentity {
    let (purpose, kind) = if create {
        (
            GrantPurpose::CreateWorkspace,
            OperationKind::CreateWorkspace,
        )
    } else {
        (GrantPurpose::OpenWorkspace, OperationKind::OpenWorkspace)
    };
    let grant = service.grant_path(purpose, path).unwrap();
    let operation = service
        .accept_workspace_operation(&id(), kind, &grant.token)
        .unwrap();
    let result = service.execute_operation(&operation.operation_id).unwrap();
    assert_eq!(result.status, OperationStatus::Succeeded);
    result.result.unwrap().identity
}

struct Fixture {
    // Drop all SQLite connections and native leases before deleting the directory.
    runtime: DesktopRuntime,
    identity: WorkspaceIdentity,
    note: Note,
    hash: String,
    preview: String,
    bytes: Vec<u8>,
    directory: TestDirectory,
}

impl Fixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let mut service = AppService::open(directory.path.join("app")).unwrap();
        let identity = workspace(&mut service, &directory.path.join("knowledge"), true);
        let mut output = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(960, 640)
            .write_to(&mut output, ImageFormat::Jpeg)
            .unwrap();
        let bytes = output.into_inner();
        let library = service.library_mut(&identity).unwrap();
        let imported = library
            .import_attachment(&id(), "thumbnail-test.jpg", &bytes)
            .unwrap();
        let note = library
            .create_document(
                &id(),
                CreateNote {
                    title: "Thumbnail authorization fixture".into(),
                    document_schema: 2,
                    document: serde_json::from_value(json!({"type":"doc", "content":[
                        {"type":"paragraph", "attrs":{"id":id()}, "content":[
                            {"type":"text", "text":"Synthetic image fixture"}
                        ]},
                        {"type":"image", "attrs":{
                            "id":id(), "blob_hash":imported.blob.hash,
                            "original_name":"thumbnail-test.jpg", "alt":"Synthetic image"
                        }}
                    ]}))
                    .unwrap(),
                    collection_id: None,
                    base_collection_row_version: None,
                },
            )
            .unwrap()
            .note;
        Self {
            runtime: DesktopRuntime::new(service).unwrap(),
            identity,
            note,
            hash: imported.blob.hash,
            preview: imported.preview_token,
            bytes,
            directory,
        }
    }

    fn path(&self, kind: &str) -> String {
        format!(
            "/{}/{}/{kind}/{}/{}",
            self.identity.local_instance_id,
            self.identity.workspace_id,
            self.note.revision_id,
            self.hash
        )
    }

    fn request(&self, path: &str, method: &str) -> Response<Vec<u8>> {
        read_image(
            &self.runtime,
            Request::builder()
                .method(method)
                .uri(path)
                .body(Vec::new())
                .unwrap(),
        )
    }

    fn blob_path(&self) -> PathBuf {
        self.directory
            .path
            .join("knowledge/blobs/sha256")
            .join(&self.hash[..2])
            .join(&self.hash)
    }
}

fn unavailable(response: Response<Vec<u8>>) {
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "text/plain");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert_eq!(response.body(), b"Image unavailable");
}

#[test]
fn thumbnail_http_returns_real_png_and_keeps_original_routes_unchanged() {
    let fixture = Fixture::new();
    let response = fixture.request(&fixture.path("thumbnail"), "GET");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert_eq!(
        image::load_from_memory(response.body())
            .unwrap()
            .dimensions(),
        (480, 320)
    );
    let preview = format!(
        "/{}/{}/preview/{}",
        fixture.identity.local_instance_id, fixture.identity.workspace_id, fixture.preview
    );
    for path in [fixture.path("revision"), preview] {
        let original = fixture.request(&path, "GET");
        assert_eq!(original.status(), StatusCode::OK);
        assert_eq!(original.headers()[header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(original.body(), &fixture.bytes);
    }
    assert_eq!(fs::read(fixture.blob_path()).unwrap(), fixture.bytes);
}

#[test]
fn thumbnail_http_rejects_extra_parameters_and_wrong_authorization() {
    let fixture = Fixture::new();
    let valid = fixture.path("thumbnail");
    // Populate the process cache before testing all alternate request identities.
    assert_eq!(fixture.request(&valid, "GET").status(), StatusCode::OK);
    for path in [
        format!("{valid}?width=10"),
        format!("{valid}?"),
        format!("{valid}/480/320"),
        format!("{valid}/"),
        format!("/{valid}"),
        valid.replace(&fixture.identity.local_instance_id, &id()),
        valid.replace(&fixture.identity.workspace_id, &id()),
        valid.replace(&fixture.note.revision_id, &id()),
        valid.replace(&fixture.hash, &"b".repeat(64)),
        valid.replace(&fixture.hash, "../image.jpg"),
        valid.replace(&fixture.note.revision_id, "not-a-revision"),
    ] {
        unavailable(fixture.request(&path, "GET"));
    }
    for method in ["POST", "HEAD", "PUT", "DELETE"] {
        unavailable(fixture.request(&valid, method));
    }
}

#[test]
fn thumbnails_follow_exact_revisions_and_preserve_authorized_history() {
    let fixture = Fixture::new();
    let mut service = fixture.runtime.service.lock().unwrap();
    let library = service.library_mut(&fixture.identity).unwrap();
    let removed = library
        .save_document(
            &id(),
            SaveNote {
                note_id: fixture.note.id.clone(),
                base_revision_id: fixture.note.revision_id.clone(),
                title: fixture.note.title.clone(),
                document_schema: 2,
                document: NoteDocument::from_text(id(), "The current revision has no image"),
            },
        )
        .unwrap()
        .note;
    library.set_trashed(&fixture.note.id, true).unwrap();
    drop(service);
    let historical = fixture.path("thumbnail");
    unavailable(fixture.request(
        &historical.replace(&fixture.note.revision_id, &removed.revision_id),
        "GET",
    ));
    assert_eq!(fixture.request(&historical, "GET").status(), StatusCode::OK);
    assert_eq!(fs::read(fixture.blob_path()).unwrap(), fixture.bytes);
}

#[test]
fn cached_thumbnails_fail_after_original_corruption_or_removal() {
    let fixture = Fixture::new();
    let path = fixture.path("thumbnail");
    assert_eq!(fixture.request(&path, "GET").status(), StatusCode::OK);
    let mut corrupted = fixture.bytes.clone();
    corrupted[40] ^= 1;
    fs::write(fixture.blob_path(), &corrupted).unwrap();
    unavailable(fixture.request(&path, "GET"));
    fs::write(fixture.blob_path(), &fixture.bytes).unwrap();
    assert_eq!(fixture.request(&path, "GET").status(), StatusCode::OK);
    fs::remove_file(fixture.blob_path()).unwrap();
    unavailable(fixture.request(&path, "GET"));
}

#[test]
fn cached_thumbnails_cannot_cross_closed_or_switched_workspaces() {
    let fixture = Fixture::new();
    let path = fixture.path("thumbnail");
    assert_eq!(fixture.request(&path, "GET").status(), StatusCode::OK);
    fixture.runtime.service.lock().unwrap().close_workspace();
    unavailable(fixture.request(&path, "GET"));
    let mut service = fixture.runtime.service.lock().unwrap();
    let other = workspace(&mut service, &fixture.directory.path.join("other"), true);
    drop(service);
    unavailable(fixture.request(&path, "GET"));
    let forged = path
        .replace(
            &fixture.identity.local_instance_id,
            &other.local_instance_id,
        )
        .replace(&fixture.identity.workspace_id, &other.workspace_id);
    unavailable(fixture.request(&forged, "GET"));
    let mut service = fixture.runtime.service.lock().unwrap();
    let reopened = workspace(
        &mut service,
        &fixture.directory.path.join("knowledge"),
        false,
    );
    assert_eq!(reopened, fixture.identity);
    drop(service);
    assert_eq!(fixture.request(&path, "GET").status(), StatusCode::OK);
}

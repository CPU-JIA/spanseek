use pkms_app::{AppService, GrantPurpose, OperationKind, SourceWorkerConfig, WorkspaceIdentity};
mod common;
use pkms_core::{ImportWebpage, JobStatus};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}");
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let stopping = stop.clone();
        let count = requests.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut input = Vec::new();
                while !input.windows(4).any(|bytes| bytes == b"\r\n\r\n") && input.len() < 8192 {
                    let mut chunk = [0; 1024];
                    let size = stream.read(&mut chunk).unwrap_or(0);
                    if size == 0 {
                        break;
                    }
                    input.extend_from_slice(&chunk[..size]);
                }
                let request = String::from_utf8_lossy(&input);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                count.fetch_add(1, Ordering::AcqRel);
                let article = "<!doctype html><title>真实网页标题</title><nav>Navigation noise</nav><main><p>Hello <b>世界😀</b> &amp; useful evidence.</p><script>no execution</script><img src='/tracking'></main>";
                let (status, extra, body) = match path {
                    path if path.starts_with("/hop") => {
                        let hop: usize = path.trim_start_matches("/hop").parse().unwrap();
                        ("302 Found", format!("Location: /hop{}\r\n", hop + 1), "")
                    }
                    "/redirect" => ("302 Found", "Location: /article\r\n".to_owned(), ""),
                    "/loop" => ("302 Found", "Location: /loop\r\n".to_owned(), ""),
                    "/credentials" => (
                        "302 Found",
                        format!("Location: http://user:pass@{address}/article\r\n"),
                        "",
                    ),
                    "/login" => ("401 Unauthorized", String::new(), "login required"),
                    "/oversize" => {
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 10485761\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                    "/chunked-oversize" => {
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        let chunk = vec![b'x'; 65536];
                        for _ in 0..161 {
                            if stream.write_all(b"10000\r\n").is_err()
                                || stream.write_all(&chunk).is_err()
                                || stream.write_all(b"\r\n").is_err()
                            {
                                break;
                            }
                        }
                        let _ = stream.write_all(b"0\r\n\r\n");
                        continue;
                    }
                    "/slow" => {
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n");
                        let started = Instant::now();
                        while started.elapsed() < Duration::from_secs(5)
                            && !stopping.load(Ordering::Acquire)
                        {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        continue;
                    }
                    _ => ("200 OK", String::new(), article),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            url,
            stop,
            requests,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn setup(root: &std::path::Path) -> (AppService, WorkspaceIdentity) {
    let mut app = AppService::open(root.join("app")).unwrap();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, root.join("knowledge"))
        .unwrap();
    let operation = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let identity = app
        .execute_operation(&operation.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity;
    (app, identity)
}
fn config() -> SourceWorkerConfig {
    SourceWorkerConfig {
        executable: common::worker_executable(),
        pdfium_library_path: None,
        ocr: None,
        shutdown: Default::default(),
    }
}
fn request(url: String) -> ImportWebpage {
    ImportWebpage {
        url,
        collection_id: None,
        add_version_to: None,
        base_row_version: None,
    }
}
#[test]
fn real_http_redirect_snapshot_is_immutable_and_worker_extracts_title_and_text() {
    let server = Server::start();
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = setup(root.path());
    let url = format!("{}/redirect", server.url);
    let command = id();
    let accepted = app
        .import_webpage(&identity, &command, request(url.clone()))
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
    let library = app.library(&identity).unwrap();
    let (version, mut original) = library
        .source_original_file(job.source_version_id.as_ref().unwrap())
        .unwrap();
    let mut html = String::new();
    original.read_to_string(&mut html).unwrap();
    assert!(html.contains("<script>no execution</script>"));
    assert_eq!(version.origin_url.as_deref(), Some(url.as_str()));
    let capture = version.web_capture.unwrap();
    assert_eq!(capture.final_url, format!("{}/article", server.url));
    assert_eq!(capture.http_status, 200);
    assert_eq!(
        library
            .source(job.source_id.as_ref().unwrap())
            .unwrap()
            .display_title,
        "真实网页标题"
    );
    let text = library.source_text_page(&version.id, None, 0, 100).unwrap();
    assert_eq!(text.blocks[0].text, "Hello 世界😀 & useful evidence.");
    assert_eq!(server.requests.load(Ordering::Acquire), 2);
    assert_eq!(
        app.import_webpage(&identity, &command, request(url))
            .unwrap()
            .job_id,
        accepted.job_id
    );
    assert_eq!(server.requests.load(Ordering::Acquire), 2);
}
#[test]
fn login_redirect_credentials_loop_and_size_limits_fail_without_fake_sources() {
    let server = Server::start();
    for (path, code) in [
        ("/login", "webpage_login_required"),
        ("/loop", "webpage_redirect_loop"),
        ("/credentials", "webpage_invalid_url"),
        ("/oversize", "webpage_size_limit"),
        ("/hop0", "webpage_redirect_limit"),
        ("/chunked-oversize", "webpage_size_limit"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (mut app, identity) = setup(root.path());
        let accepted = app
            .import_webpage(&identity, &id(), request(format!("{}{path}", server.url)))
            .unwrap();
        app.source_jobs_worker()
            .unwrap()
            .unwrap()
            .run(config())
            .unwrap();
        let job = app
            .library(&identity)
            .unwrap()
            .source_job(&accepted.job_id)
            .unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error_code.as_deref(), Some(code), "{path}");
        assert!(job.source_version_id.is_none());
    }
}
#[test]
fn slow_response_is_cancellable_without_waiting_for_the_network_timeout() {
    let server = Server::start();
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = setup(root.path());
    let accepted = app
        .import_webpage(&identity, &id(), request(format!("{}/slow", server.url)))
        .unwrap();
    let worker = app.source_jobs_worker().unwrap().unwrap();
    let thread = std::thread::spawn(move || worker.run(config()));
    let started = Instant::now();
    while server.requests.load(Ordering::Acquire) == 0 {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    app.library_mut(&identity)
        .unwrap()
        .cancel_source_job(&accepted.job_id)
        .unwrap();
    let canceled = Instant::now();
    thread.join().unwrap().unwrap();
    assert!(canceled.elapsed() < Duration::from_secs(2));
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .source_job(&accepted.job_id)
            .unwrap()
            .status,
        JobStatus::Canceled
    );
}
#[test]
fn rejects_non_http_schemes_and_embedded_credentials_before_acceptance() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = setup(root.path());
    for url in [
        "file:///C:/private",
        "javascript:alert(1)",
        "http://user:secret@example.com",
    ] {
        assert!(
            app.import_webpage(&identity, &id(), request(url.into()))
                .is_err()
        );
    }
    assert!(
        app.library(&identity)
            .unwrap()
            .source_jobs(100)
            .unwrap()
            .is_empty()
    );
}

#[test]
#[ignore = "requires public HTTPS connectivity; run explicitly for release evidence"]
fn public_https_snapshot_preserves_real_response_and_extracts_visible_text() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = setup(root.path());
    let accepted = app
        .import_webpage(&identity, &id(), request("https://example.com/".into()))
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
    let library = app.library(&identity).unwrap();
    let version = library
        .source_version(job.source_version_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(version.web_capture.as_ref().unwrap().http_status, 200);
    assert!(
        version
            .web_capture
            .unwrap()
            .final_url
            .starts_with("https://")
    );
    let text = library.source_text_page(&version.id, None, 0, 100).unwrap();
    assert_eq!(
        library
            .source(job.source_id.as_ref().unwrap())
            .unwrap()
            .display_title,
        "Example Domain"
    );
    assert!(
        text.blocks
            .iter()
            .any(|block| block.text.contains("documentation examples"))
    );
}

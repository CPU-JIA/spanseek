use crate::worker_process::WorkerChild;
use crate::{AppError, Result, inference_resources::heavy_resource_slot};
use pkms_inference::protocol::*;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct InferenceConfig {
    pub executable: PathBuf,
    pub model_directory: PathBuf,
    pub vector_root: PathBuf,
    pub shutdown: Arc<AtomicBool>,
}
#[derive(PartialEq, Eq)]
struct SessionKey {
    executable: PathBuf,
    model: PathBuf,
    vectors: PathBuf,
}
type ReaderItem = std::result::Result<WorkerEvent, &'static str>;
type InputWriter = std::thread::JoinHandle<(std::process::ChildStdin, std::io::Result<()>)>;
struct Session {
    key: SessionKey,
    child: WorkerChild,
    stdin: Option<std::process::ChildStdin>,
    receive: Option<mpsc::Receiver<ReaderItem>>,
    reader: Option<std::thread::JoinHandle<()>>,
    writer: Option<InputWriter>,
    last_used: Instant,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Wake a bounded-channel producer before joining it.
        self.receive.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        self.stdin.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
static SESSION: std::sync::Mutex<Option<Session>> = std::sync::Mutex::new(None);
static REAPER: std::sync::Once = std::sync::Once::new();
static STOP_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Idle workers are destroyed immediately; an active request observes the epoch
/// within its normal 100 ms cancellation poll and destroys its own worker.
pub fn stop_inference_session() {
    STOP_EPOCH.fetch_add(1, Ordering::AcqRel);
    let idle = SESSION.lock().unwrap_or_else(|e| e.into_inner()).take();
    drop(idle);
}
fn start_reaper() {
    REAPER.call_once(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let expired = {
                    let mut idle = SESSION.lock().unwrap_or_else(|e| e.into_inner());
                    if idle
                        .as_ref()
                        .is_some_and(|s| s.last_used.elapsed() >= IDLE_TIMEOUT)
                    {
                        idle.take()
                    } else {
                        None
                    }
                };
                drop(expired);
            }
        });
    });
}
impl Session {
    fn spawn(key: SessionKey) -> Result<Self> {
        let mut command = Command::new(&key.executable);
        command
            .arg("--session")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let child = WorkerChild::spawn(&mut command)?;
        let mut session = Self {
            key,
            child,
            stdin: None,
            receive: None,
            reader: None,
            writer: None,
            last_used: Instant::now(),
        };
        session.stdin = session.child.stdin.take();
        let stdout = session
            .child
            .stdout
            .take()
            .ok_or(AppError::InvalidInput("inference_pipe_failed"))?;
        let (send, receive) = mpsc::sync_channel(8);
        session.receive = Some(receive);
        session.reader = Some(std::thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            loop {
                let item = match line(&mut input) {
                    Ok(Some(bytes)) => serde_json::from_slice::<WorkerEvent>(&bytes)
                        .map_err(|_| "inference_protocol_error"),
                    Ok(None) => break,
                    Err(_) => Err("inference_event_limit"),
                };
                let failed = item.is_err();
                if send.send(item).is_err() || failed {
                    break;
                }
            }
        }));
        Ok(session)
    }
    fn write(&mut self, bytes: Vec<u8>) -> Result<()> {
        let mut stdin = self
            .stdin
            .take()
            .ok_or(AppError::InvalidInput("inference_pipe_failed"))?;
        self.writer = Some(std::thread::spawn(move || {
            let result = stdin.write_all(&bytes).and_then(|_| stdin.flush());
            (stdin, result)
        }));
        Ok(())
    }
    fn poll_writer(&mut self) -> Result<bool> {
        if self
            .writer
            .as_ref()
            .is_some_and(|writer| writer.is_finished())
        {
            let (stdin, result) = self
                .writer
                .take()
                .expect("finished writer")
                .join()
                .map_err(|_| AppError::InvalidInput("inference_pipe_failed"))?;
            self.stdin = Some(stdin);
            result?;
        }
        Ok(self.writer.is_none())
    }
}
fn request_frame(request: &WorkerRequest) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(request)?;
    if bytes.len() as u64 >= MAX_REQUEST_BYTES {
        return Err(AppError::InvalidInput("inference_request_limit"));
    }
    bytes.push(b'\n');
    Ok(bytes)
}
fn line(reader: &mut impl BufRead) -> std::io::Result<Option<Vec<u8>>> {
    let mut result = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if result.is_empty() {
                Ok(None)
            } else {
                Err(std::io::Error::other("incomplete event"))
            };
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(buffer.len(), |index| index + 1);
        if result.len() + count > MAX_EVENT_BYTES + 1 {
            return Err(std::io::Error::other("event limit"));
        }
        result.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if newline.is_some() {
            return Ok(Some(result));
        }
    }
}
pub(crate) fn failure_code(code: &str) -> &'static str {
    match code {
        "embedding_batch_limit" => "embedding_batch_limit",
        "embedding_chunk_limit" => "embedding_chunk_limit",
        "embedding_chunk_size_limit" => "embedding_chunk_size_limit",
        "embedding_inference_failed" => "embedding_inference_failed",
        "embedding_input_limit" => "embedding_input_limit",
        "embedding_invalid_input" => "embedding_invalid_input",
        "embedding_invalid_plan" => "embedding_invalid_plan",
        "embedding_invalid_vector" => "embedding_invalid_vector",
        "embedding_invalid_vector_count" => "embedding_invalid_vector_count",
        "embedding_model_hash_mismatch" => "embedding_model_hash_mismatch",
        "embedding_model_invalid" => "embedding_model_invalid",
        "embedding_model_load_failed" => "embedding_model_load_failed",
        "embedding_model_unavailable" => "embedding_model_unavailable",
        "embedding_profile_mismatch" => "embedding_profile_mismatch",
        "embedding_title_token_limit" => "embedding_title_token_limit",
        "embedding_token_budget" => "embedding_token_budget",
        "embedding_tokenization_failed" => "embedding_tokenization_failed",
        "embedding_tokenizer_invalid" => "embedding_tokenizer_invalid",
        "inference_event_limit" => "inference_event_limit",
        "inference_invalid_frame" => "inference_invalid_frame",
        "inference_invalid_request" => "inference_invalid_request",
        "inference_output_unavailable" => "inference_output_unavailable",
        "inference_protocol_error" => "inference_protocol_error",
        "inference_result_invalid" => "inference_result_invalid",
        "inference_result_limit" => "inference_result_limit",
        "inference_result_unavailable" => "inference_result_unavailable",
        "inference_runtime_unavailable" => "inference_runtime_unavailable",
        "model_api_error" => "model_api_error",
        "model_authentication_failed" => "model_authentication_failed",
        "model_canceled" => "model_canceled",
        "model_context_limit" => "model_context_limit",
        "model_first_token_timeout" => "model_first_token_timeout",
        "model_idle_timeout" => "model_idle_timeout",
        "model_incomplete" => "model_incomplete",
        "model_invalid_endpoint" => "model_invalid_endpoint",
        "model_invalid_request" => "model_invalid_request",
        "model_invalid_response" => "model_invalid_response",
        "model_not_found" => "model_not_found",
        "model_rate_limited" => "model_rate_limited",
        "model_request_rejected" => "model_request_rejected",
        "model_response_too_large" => "model_response_too_large",
        "model_total_timeout" => "model_total_timeout",
        "model_unavailable" => "model_unavailable",
        "ollama_model_unavailable" => "ollama_model_unavailable",
        "ollama_timeout" => "ollama_timeout",
        "ollama_unavailable" => "ollama_unavailable",
        "projection_incomplete" => "projection_incomplete",
        "vector_commit_failed" => "vector_commit_failed",
        "vector_open_failed" => "vector_open_failed",
        "vector_query_failed" => "vector_query_failed",
        _ => "inference_failed",
    }
}
/// Callbacks persist authoritative deltas/check leases. None is the regular cancellation heartbeat.
pub(crate) fn run(
    config: &InferenceConfig,
    mut request: WorkerRequest,
    timeout: Duration,
    event: &mut dyn FnMut(Option<&Event>) -> Result<()>,
) -> Result<WorkerResult> {
    let started = Instant::now();
    let epoch = STOP_EPOCH.load(Ordering::Acquire);
    let mut check = || {
        if started.elapsed() >= timeout {
            return Err(AppError::InvalidInput("inference_timeout"));
        }
        if config.shutdown.load(Ordering::Acquire) || STOP_EPOCH.load(Ordering::Acquire) != epoch {
            return Err(AppError::InvalidInput("worker_interrupted"));
        }
        event(None)
    };
    let _permit = heavy_resource_slot().acquire(&mut check)?;
    check()?;
    if !config.executable.is_absolute() || !config.executable.is_file() {
        return Err(AppError::InvalidInput("inference_unavailable"));
    }
    if !request.model_directory.is_absolute() || !request.vector_directory.is_absolute() {
        return Err(AppError::InvalidInput("inference_invalid_request"));
    }
    // Managed directories may not exist yet (including when chat needs no embedder).
    std::fs::create_dir_all(&request.model_directory)?;
    std::fs::create_dir_all(&request.vector_directory)?;
    request.model_directory = request.model_directory.canonicalize()?;
    request.vector_directory = request.vector_directory.canonicalize()?;
    let key = SessionKey {
        executable: config.executable.canonicalize()?,
        model: request.model_directory.clone(),
        vectors: request.vector_directory.clone(),
    };
    let bytes = request_frame(&request)?;
    start_reaper();
    // The resource permit serializes active requests. Never hold this mutex while
    // doing I/O, callbacks or child teardown, so stop/reaper remain responsive.
    let mut cached = SESSION.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(previous) = &mut cached {
        let reusable = previous.key == key
            && previous.last_used.elapsed() < IDLE_TIMEOUT
            && previous.child.try_wait()?.is_none()
            && matches!(
                previous
                    .receive
                    .as_ref()
                    .expect("session reader")
                    .try_recv(),
                Err(mpsc::TryRecvError::Empty)
            );
        if !reusable {
            drop(cached.take());
        }
    }
    let mut session = match cached {
        Some(cached) => cached,
        None => Session::spawn(key)?,
    };
    session.write(bytes)?;
    let mut artifact = None;
    loop {
        if config.shutdown.load(Ordering::Acquire) || STOP_EPOCH.load(Ordering::Acquire) != epoch {
            return Err(AppError::InvalidInput("worker_interrupted"));
        }
        event(None)?;
        if started.elapsed() >= timeout {
            return Err(AppError::InvalidInput("inference_timeout"));
        }
        let written = session.poll_writer()?;
        if artifact.is_some() && written {
            break;
        }
        match session
            .receive
            .as_ref()
            .expect("session reader")
            .recv_timeout(Duration::from_millis(100))
        {
            Ok(item) => {
                let item = item.map_err(AppError::InvalidInput)?;
                if item.protocol != PROTOCOL_VERSION
                    || item.identity != request.identity
                    || artifact.is_some()
                {
                    return Err(AppError::InvalidInput("inference_identity_mismatch"));
                }
                match &item.event {
                    Event::Failed { code } => {
                        return Err(AppError::InvalidInput(failure_code(code)));
                    }
                    Event::Completed {
                        result_path,
                        sha256,
                        bytes,
                    } => artifact = Some((result_path.clone(), sha256.clone(), *bytes)),
                    _ => event(Some(&item.event))?,
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(AppError::InvalidInput("inference_incomplete"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    let result = read_artifact(&request, artifact)?;
    event(None)?;
    if started.elapsed() >= timeout {
        return Err(AppError::InvalidInput("inference_timeout"));
    }
    // Only a verified result allows reuse. No request/result is cached.
    session.last_used = Instant::now();
    let mut idle = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    if !config.shutdown.load(Ordering::Acquire) && STOP_EPOCH.load(Ordering::Acquire) == epoch {
        *idle = Some(session);
    } else {
        drop(idle);
        return Err(AppError::InvalidInput("worker_interrupted"));
    }
    Ok(result)
}
fn read_artifact(
    request: &WorkerRequest,
    artifact: Option<(PathBuf, String, u64)>,
) -> Result<WorkerResult> {
    let (path, expected_hash, expected_size) =
        artifact.ok_or(AppError::InvalidInput("inference_incomplete"))?;
    if path != request.result_path || expected_size > MAX_RESULT_BYTES as u64 {
        return Err(AppError::InvalidInput("inference_result_limit"));
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AppError::InvalidInput("inference_result_path"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(AppError::InvalidInput("inference_result_path"));
        }
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_RESULT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != expected_size
        || format!("{:x}", Sha256::digest(&bytes)) != expected_hash
    {
        return Err(AppError::InvalidInput("inference_result_hash"));
    }
    let result: WorkerResult = serde_json::from_slice(&bytes)?;
    if !matches!(
        (&request.operation, &result),
        (
            Operation::PrepareChunks { .. },
            WorkerResult::Prepared { .. }
        ) | (
            Operation::EmbedGeneration { .. } | Operation::PublishVectors { .. },
            WorkerResult::Indexed { .. }
        ) | (
            Operation::SearchVectors { .. } | Operation::SearchEmbedded { .. },
            WorkerResult::Candidates { .. }
        ) | (
            Operation::ScoreExactQuotes { .. },
            WorkerResult::QuoteScores { .. }
        ) | (Operation::Chat { .. }, WorkerResult::Answer { .. })
    ) {
        return Err(AppError::InvalidInput("inference_result_kind"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(path: PathBuf) -> WorkerRequest {
        WorkerRequest {
            protocol: PROTOCOL_VERSION,
            identity: RequestIdentity {
                request_id: "request".into(),
                job_id: "job".into(),
                attempt: 1,
                lease_token: "lease".into(),
            },
            model_directory: path.clone(),
            vector_directory: path.clone(),
            result_path: path,
            operation: Operation::Chat {
                endpoint: "http://127.0.0.1:11434".into(),
                model: "local".into(),
                messages: vec![],
                num_ctx: 4096,
                num_predict: 64,
                temperature: 0.0,
                format: None,
            },
        }
    }
    #[test]
    fn request_frame_counts_terminating_newline_in_limit() {
        let mut request = request(PathBuf::from("result"));
        let initial = serde_json::to_vec(&request).unwrap().len();
        request
            .identity
            .request_id
            .push_str(&"x".repeat(MAX_REQUEST_BYTES as usize - initial - 1));
        let frame = request_frame(&request).unwrap();
        assert_eq!(frame.len() as u64, MAX_REQUEST_BYTES);
        assert_eq!(frame.last(), Some(&b'\n'));
        drop(frame);
        request.identity.request_id.push('x');
        assert!(matches!(
            request_frame(&request),
            Err(AppError::InvalidInput("inference_request_limit"))
        ));
    }
    #[test]
    fn artifact_must_match_current_request_path_size_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut request = request(dir.path().join("result.json"));
        request.operation = Operation::SearchVectors {
            profile: pkms_core::EmbeddingProfile {
                id: "fixture".into(),
                spec: pkms_inference::profile::profile_spec(),
            },
            generations: vec![],
            query: "fixture".into(),
            limit: 1,
        };
        let bytes = serde_json::to_vec(&WorkerResult::Candidates { items: vec![] }).unwrap();
        std::fs::write(&request.result_path, &bytes).unwrap();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let artifact = (
            request.result_path.clone(),
            hash.clone(),
            bytes.len() as u64,
        );
        assert!(
            matches!(read_artifact(&request, Some(artifact)), Ok(WorkerResult::Candidates { items }) if items.is_empty())
        );
        assert!(
            read_artifact(
                &request,
                Some((
                    dir.path().join("other.json"),
                    hash.clone(),
                    bytes.len() as u64
                ))
            )
            .is_err()
        );
        assert!(
            read_artifact(
                &request,
                Some((request.result_path.clone(), hash, bytes.len() as u64 + 1))
            )
            .is_err()
        );
        assert!(
            read_artifact(
                &request,
                Some((
                    request.result_path.clone(),
                    "0".repeat(64),
                    bytes.len() as u64
                ))
            )
            .is_err()
        );
    }
    #[cfg(windows)]
    #[test]
    fn session_drop_reaps_child_and_unblocks_backpressured_reader() {
        let child = WorkerChild::spawn(
            Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 60",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        send.send(Err("fixture_backpressure")).unwrap();
        let (finished, completion) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let _ = send.send(Err("fixture_backpressure"));
            finished.send(()).unwrap();
        });
        let session = Session {
            key: SessionKey {
                executable: PathBuf::new(),
                model: PathBuf::new(),
                vectors: PathBuf::new(),
            },
            child,
            stdin: None,
            receive: Some(receive),
            reader: Some(reader),
            writer: None,
            last_used: Instant::now(),
        };
        let start = Instant::now();
        drop(session);
        assert!(start.elapsed() < Duration::from_secs(5));
        completion.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    #[test]
    fn exact_quote_artifact_requires_matching_operation() {
        let dir = tempfile::tempdir().unwrap();
        let mut request = request(dir.path().join("quotes.json"));
        let bytes = serde_json::to_vec(&WorkerResult::QuoteScores { items: vec![] }).unwrap();
        std::fs::write(&request.result_path, &bytes).unwrap();
        let artifact = (
            request.result_path.clone(),
            format!("{:x}", Sha256::digest(&bytes)),
            bytes.len() as u64,
        );
        assert!(matches!(
            read_artifact(&request, Some(artifact.clone())),
            Err(AppError::InvalidInput("inference_result_kind"))
        ));
        request.operation = Operation::ScoreExactQuotes {
            profile: pkms_core::EmbeddingProfile {
                id: "fixture".into(),
                spec: pkms_inference::profile::profile_spec(),
            },
            query: "fixture".into(),
            inputs: vec![],
            limit: 1,
        };
        assert!(matches!(
            read_artifact(&request, Some(artifact)),
            Ok(WorkerResult::QuoteScores { items }) if items.is_empty()
        ));
    }
    #[test]
    fn remote_vector_operations_require_their_exact_result_kind() {
        let dir = tempfile::tempdir().unwrap();
        let mut request = request(dir.path().join("remote.json"));
        let profile = pkms_core::EmbeddingProfile {
            id: "remote".into(),
            spec: pkms_inference::remote_chunks::default_profile_spec(
                "https://example.com/ai/v1".into(),
            ),
        };
        request.operation = Operation::SearchEmbedded {
            profile: profile.clone(),
            generations: vec![],
            vector: vec![1.; 1024],
            limit: 1,
        };
        let candidates = WorkerResult::Candidates { items: vec![] };
        let write = |result: &WorkerResult| {
            let bytes = serde_json::to_vec(result).unwrap();
            std::fs::write(&request.result_path, &bytes).unwrap();
            (
                request.result_path.clone(),
                format!("{:x}", Sha256::digest(&bytes)),
                bytes.len() as u64,
            )
        };
        assert!(matches!(
            read_artifact(&request, Some(write(&candidates))),
            Ok(WorkerResult::Candidates { .. })
        ));
        let indexed = WorkerResult::Indexed {
            publish: pkms_core::IndexPublish {
                generation_id: "g".into(),
                lease_token: "lease".into(),
                profile_id: profile.id.clone(),
                chunk_count: 1,
            },
        };
        let receipt = write(&indexed);
        assert!(matches!(
            read_artifact(&request, Some(receipt.clone())),
            Err(AppError::InvalidInput("inference_result_kind"))
        ));
        request.operation = Operation::PublishVectors {
            profile,
            plan: pkms_core::IndexPlan {
                profile_id: "remote".into(),
                object_id: "o".into(),
                content_revision: "r".into(),
                metadata_revision: 1,
                parser_revision: "p".into(),
                chunks: vec![],
            },
            generation_id: "g".into(),
            vectors: vec![],
        };
        assert!(matches!(
            read_artifact(&request, Some(receipt)),
            Ok(WorkerResult::Indexed { .. })
        ));
    }
    #[test]
    fn inference_deadline_includes_waiting_for_resource_slot() {
        let permit = heavy_resource_slot().acquire(&mut || Ok(())).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = InferenceConfig {
            executable: dir.path().join("absent-runtime.exe"),
            model_directory: dir.path().join("models"),
            vector_root: dir.path().join("vectors"),
            shutdown: Arc::new(AtomicBool::new(false)),
        };
        let result = run(
            &config,
            request(dir.path().join("result.json")),
            Duration::from_millis(20),
            &mut |_| Ok(()),
        );
        assert!(matches!(
            result,
            Err(AppError::InvalidInput("inference_timeout"))
        ));
        drop(permit);
    }
    #[test]
    #[ignore = "requires PKMS_INFERENCE_EXE pointing to the built runtime binary"]
    fn real_session_reuses_process_and_replaces_workspace() {
        let executable = std::env::var_os("PKMS_INFERENCE_EXE").expect("runtime binary path");
        let temp = tempfile::tempdir().unwrap();
        let config = InferenceConfig {
            executable: PathBuf::from(executable),
            model_directory: temp.path().join("models"),
            vector_root: temp.path().join("vectors"),
            shutdown: Arc::new(AtomicBool::new(false)),
        };
        stop_inference_session();
        let mut pids = Vec::new();
        for (index, workspace) in ["first", "first", "second"].into_iter().enumerate() {
            let mut request = request(temp.path().join(format!("result-{index}.json")));
            request.identity.request_id = format!("request-{index}");
            request.model_directory = config.model_directory.clone();
            request.vector_directory = config.vector_root.join(workspace);
            request.operation = Operation::SearchVectors {
                profile: pkms_core::EmbeddingProfile {
                    id: "fixture".into(),
                    spec: pkms_inference::profile::profile_spec(),
                },
                generations: vec![],
                query: "fixture".into(),
                limit: 1,
            };
            assert!(
                matches!(run(&config, request, Duration::from_secs(30), &mut |_| Ok(())), Ok(WorkerResult::Candidates { items }) if items.is_empty())
            );
            pids.push(
                SESSION
                    .lock()
                    .unwrap()
                    .as_ref()
                    .expect("cached session")
                    .child
                    .id(),
            );
        }
        stop_inference_session();
        assert_eq!(pids[0], pids[1]);
        assert_ne!(pids[1], pids[2]);
        assert!(SESSION.lock().unwrap().is_none());
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "requires PKMS_INFERENCE_EXE pointing to the built runtime binary"]
    fn hard_parent_exit_kills_real_inference_session() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "inference_process::tests::parent_exit_fixture",
                "--ignored",
            ])
            .env("PKMS_WORKER_LIFETIME_DIRECTORY", directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::worker_process::tests::assert_parent_kill(
            &mut command,
            &directory.path().join("inference.pid"),
        );
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "subprocess fixture; run only through lifecycle tests"]
    fn parent_exit_fixture() {
        let executable =
            PathBuf::from(std::env::var_os("PKMS_INFERENCE_EXE").expect("runtime binary path"));
        let directory = PathBuf::from(std::env::var_os("PKMS_WORKER_LIFETIME_DIRECTORY").unwrap());
        let session = Session::spawn(SessionKey {
            executable,
            model: directory.join("models"),
            vectors: directory.join("vectors"),
        })
        .unwrap();
        std::fs::write(
            directory.join("inference.pid"),
            session.child.id().to_string(),
        )
        .unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    #[test]
    fn events_require_bounded_complete_lines() {
        assert_eq!(
            line(&mut std::io::Cursor::new(b"{}\n{}")).unwrap(),
            Some(b"{}\n".to_vec())
        );
        assert!(line(&mut std::io::Cursor::new(vec![b'x'; MAX_EVENT_BYTES + 2])).is_err());
        assert!(line(&mut std::io::Cursor::new(b"{}")).is_err());
    }
}

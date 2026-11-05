#![cfg(feature = "runtime")]
use pkms_inference::{profile::profile_spec, protocol::*};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

// Explicitly exercise an immutable QA snapshot when checking packaged/runtime
// artifacts; Cargo's top-level binary path can still refer to an older build.
fn inference_executable() -> std::path::PathBuf {
    std::env::var_os("PKMS_INFERENCE_TEST_EXE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_pkms-inference").into())
}

#[test]
fn session_frames_two_requests_without_identity_or_result_crossover() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = Command::new(inference_executable())
        .arg("--session")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    for index in 0..2 {
        let request = WorkerRequest {
            protocol: PROTOCOL_VERSION,
            identity: RequestIdentity {
                request_id: format!("request-{index}"),
                job_id: format!("job-{index}"),
                attempt: 1,
                lease_token: format!("lease-{index}"),
            },
            model_directory: temp.path().to_owned(),
            vector_directory: temp.path().join("vectors"),
            result_path: temp.path().join(format!("result-{index}.json")),
            operation: Operation::SearchVectors {
                profile: pkms_core::EmbeddingProfile {
                    id: "profile".into(),
                    spec: profile_spec(),
                },
                generations: Vec::new(),
                query: "empty scope needs no model".into(),
                limit: 5,
            },
        };
        serde_json::to_writer(&mut input, &request).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0);
        let event: WorkerEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(event.identity, request.identity);
        let Event::Completed {
            result_path,
            bytes,
            sha256,
        } = event.event
        else {
            panic!("expected completion")
        };
        assert_eq!(result_path, request.result_path);
        let result = std::fs::read(result_path).unwrap();
        assert_eq!(bytes as usize, result.len());
        use sha2::Digest;
        assert_eq!(sha256, format!("{:x}", sha2::Sha256::digest(&result)));
        assert!(
            matches!(serde_json::from_slice::<WorkerResult>(&result).unwrap(), WorkerResult::Candidates { items } if items.is_empty())
        );
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn session_rejects_an_unterminated_frame_instead_of_executing_it() {
    let mut child = Command::new(inference_executable())
        .arg("--session")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{}").unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(2));
}

#[test]
fn session_rejects_legacy_protocol_and_invalid_format_without_publication() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = Command::new(inference_executable())
        .arg("--session")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    for (index, (protocol, citation, code)) in [
        (1, "E1", "inference_invalid_request"),
        (PROTOCOL_VERSION + 1, "E1", "inference_invalid_request"),
        (PROTOCOL_VERSION, "E33", "model_invalid_request"),
    ]
    .into_iter()
    .enumerate()
    {
        let request = WorkerRequest {
            protocol,
            identity: RequestIdentity {
                request_id: format!("reject-{index}"),
                job_id: "job".into(),
                attempt: 1,
                lease_token: "lease".into(),
            },
            model_directory: temp.path().to_owned(),
            vector_directory: temp.path().join("vectors"),
            result_path: temp.path().join(format!("result-{index}.json")),
            operation: Operation::Chat {
                endpoint: "http://127.0.0.1:1".into(),
                model: "fixture-not-contacted".into(),
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: "synthetic protocol test".into(),
                }],
                num_ctx: 2048,
                num_predict: 128,
                temperature: 0.0,
                format: Some(ChatFormat::EvidenceAnswer {
                    citation_ids: vec![citation.into()],
                }),
            },
        };
        serde_json::to_writer(&mut input, &request).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0);
        let event: WorkerEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(event.protocol, PROTOCOL_VERSION);
        assert_eq!(event.identity, request.identity);
        assert!(matches!(event.event, Event::Failed { code: actual } if actual == code));
        assert!(!request.result_path.exists());
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
#[ignore = "requires pinned real E5 files; runs real ONNX and Lance in one resident process"]
fn real_session_indexes_then_queries_with_the_resident_model() {
    use pkms_core::{EmbeddingProfile, EvidenceKind, EvidenceTarget, IndexInput, IndexInputBlock};
    let temp = tempfile::tempdir().unwrap();
    let model_directory: std::path::PathBuf = std::env::var_os("PKMS_E5_MODEL_DIR")
        .expect("set PKMS_E5_MODEL_DIR")
        .into();
    let profile = EmbeddingProfile {
        id: "real-e5-session-smoke".into(),
        spec: profile_spec(),
    };
    let mut child = Command::new(inference_executable())
        .arg("--session")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut sequence = 0;
    let model_loads = std::cell::Cell::new(0);
    let mut send = |operation| {
        sequence += 1;
        let request = WorkerRequest {
            protocol: PROTOCOL_VERSION,
            identity: RequestIdentity {
                request_id: format!("request-{sequence}"),
                job_id: "real-session".into(),
                attempt: 1,
                lease_token: "test-lease".into(),
            },
            model_directory: model_directory.clone(),
            vector_directory: temp.path().join("vectors"),
            result_path: temp.path().join(format!("result-{sequence}.json")),
            operation,
        };
        serde_json::to_writer(&mut input, &request).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(output.read_line(&mut line).unwrap() > 0);
            let event: WorkerEvent = serde_json::from_str(&line).unwrap();
            assert_eq!(event.identity, request.identity);
            match event.event {
                Event::Completed {
                    result_path,
                    sha256,
                    bytes,
                } => {
                    let result = std::fs::read(result_path).unwrap();
                    use sha2::Digest;
                    assert_eq!(bytes as usize, result.len());
                    assert_eq!(sha256, format!("{:x}", sha2::Sha256::digest(&result)));
                    break Ok(serde_json::from_slice::<WorkerResult>(&result).unwrap());
                }
                Event::Failed { code } => break Err(code),
                Event::Progress { stage, .. } if stage == "loading_model" => {
                    model_loads.set(model_loads.get() + 1)
                }
                _ => {}
            }
        }
    };
    let text = "保留原始证据及不可变来源版本，每一条引用都能够回查。";
    let prepared = send(Operation::PrepareChunks {
        profile: profile.clone(),
        input: IndexInput {
            object_id: "note".into(),
            kind: EvidenceKind::Note,
            content_revision: "1".into(),
            metadata_revision: 1,
            parser_revision: "smoke".into(),
            title: "证据保全".into(),
            blocks: vec![IndexInputBlock {
                text: text.into(),
                target: EvidenceTarget {
                    object_id: "note".into(),
                    kind: EvidenceKind::Note,
                    content_revision: "1".into(),
                    metadata_revision: 1,
                    parse_id: None,
                    block_id: "block".into(),
                    start_codepoint: 0,
                    end_codepoint: text.chars().count() as u32,
                    page_index: None,
                },
            }],
        },
    });
    let WorkerResult::Prepared { plan } = prepared.unwrap() else {
        panic!("expected plan")
    };
    let chunk_count = plan.chunks.len() as u32;
    let started = std::time::Instant::now();
    assert!(matches!(
        send(Operation::EmbedGeneration {
            profile: profile.clone(),
            plan,
            title: "证据保全".into(),
            generation_id: "generation".into()
        })
        .unwrap(),
        WorkerResult::Indexed { .. }
    ));
    eprintln!("session_cold_index_ms={}", started.elapsed().as_millis());
    assert_eq!(
        send(Operation::SearchVectors {
            profile: profile.clone(),
            generations: vec![GenerationExpectation {
                generation_id: "generation".into(),
                chunk_count
            }],
            query: "超预算输入不能令下一次查询重载模型。".repeat(1000),
            limit: 5,
        })
        .err()
        .unwrap(),
        "embedding_token_budget"
    );
    let mut samples = Vec::new();
    for _ in 0..50 {
        let started = std::time::Instant::now();
        let result = send(Operation::SearchVectors {
            profile: profile.clone(),
            generations: vec![GenerationExpectation {
                generation_id: "generation".into(),
                chunk_count,
            }],
            query: "如何保存原始证据？".into(),
            limit: 5,
        });
        let WorkerResult::Candidates { items } = result.unwrap() else {
            panic!("expected candidates")
        };
        assert_eq!(items.len(), chunk_count as usize);
        assert_eq!(items[0].object_id, "note");
        assert_eq!(items[0].generation_id, "generation");
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    eprintln!("resident_pipeline_ms={samples:?}");
    assert_eq!(model_loads.get(), 1);
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn resident_bge_search_recovers_after_failure_and_reads_external_updates_and_switches() {
    use pkms_inference::{
        remote_chunks,
        vector::{VectorProjection, VectorRow},
    };
    use std::{sync::mpsc, time::Duration};
    struct OwnedChild(std::process::Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let profile = pkms_core::EmbeddingProfile {
        id: "synthetic-bge".into(),
        spec: remote_chunks::default_profile_spec("https://example.invalid/v1".into()),
    };
    let write = |directory: &std::path::Path, generation: &str, chunk: &str| {
        runtime.block_on(async {
            VectorProjection::open(directory)
                .await
                .unwrap()
                .publish(
                    &profile.id,
                    generation,
                    1024,
                    &[VectorRow {
                        chunk_id: chunk.into(),
                        object_id: format!("object-{chunk}"),
                        content_revision: "1".into(),
                        vector: vec![1.; 1024],
                    }],
                )
                .await
                .unwrap();
        });
    };
    write(&first, "old", "first-old");
    write(&second, "old", "second-old");
    let mut command = Command::new(inference_executable());
    command
        .arg("--session")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = OwnedChild(command.spawn().unwrap());
    let mut input = child.0.stdin.take().unwrap();
    let output = child.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            let event: WorkerEvent = serde_json::from_str(&line.unwrap()).unwrap();
            if sender.send(event).is_err() {
                break;
            }
        }
    });
    let mut send = |index: usize, directory: &std::path::Path, generation: &str, count: u32| {
        let request = WorkerRequest {
            protocol: PROTOCOL_VERSION,
            identity: RequestIdentity {
                request_id: format!("request-{index}"),
                job_id: "job".into(),
                attempt: 1,
                lease_token: format!("lease-{index}"),
            },
            model_directory: temp.path().to_owned(),
            vector_directory: directory.to_owned(),
            result_path: temp.path().join(format!("result-{index}.json")),
            operation: Operation::SearchEmbedded {
                profile: profile.clone(),
                generations: vec![GenerationExpectation {
                    generation_id: generation.into(),
                    chunk_count: count,
                }],
                vector: vec![1.; 1024],
                limit: 2,
            },
        };
        serde_json::to_writer(&mut input, &request).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
        loop {
            let event = receiver
                .recv_timeout(Duration::from_secs(20))
                .expect("worker did not reach terminal state");
            assert_eq!(event.identity, request.identity);
            match event.event {
                Event::Completed {
                    result_path,
                    sha256,
                    bytes,
                } => {
                    let raw = std::fs::read(result_path).unwrap();
                    use sha2::Digest;
                    assert_eq!(raw.len(), bytes as usize);
                    assert_eq!(format!("{:x}", sha2::Sha256::digest(&raw)), sha256);
                    let WorkerResult::Candidates { items } = serde_json::from_slice(&raw).unwrap()
                    else {
                        panic!("wrong result")
                    };
                    break Ok(items);
                }
                Event::Failed { code } => {
                    assert!(!request.result_path.exists());
                    break Err(code);
                }
                _ => {}
            }
        }
    };
    assert_eq!(send(0, &first, "old", 1).unwrap()[0].chunk_id, "first-old");
    assert_eq!(
        send(1, &first, "old", 2).unwrap_err(),
        "projection_incomplete"
    );
    write(&first, "new", "first-new");
    assert_eq!(send(2, &first, "new", 1).unwrap()[0].chunk_id, "first-new");
    assert_eq!(
        send(3, &second, "old", 1).unwrap()[0].chunk_id,
        "second-old"
    );
    assert_eq!(send(4, &first, "old", 1).unwrap()[0].chunk_id, "first-old");
    drop(input);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "worker failed to exit on EOF"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    reader.join().unwrap();
}

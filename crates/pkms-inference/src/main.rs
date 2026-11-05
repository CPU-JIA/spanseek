use pkms_inference::{
    embedding::{Chunker, DIMENSIONS, Embedder},
    ollama,
    protocol::*,
    vector::{VectorConnectionCache, VectorRow},
};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Default)]
struct ModelCache {
    model: Option<(PathBuf, String, Embedder)>,
    tokenizer: Option<(PathBuf, Chunker)>,
    vectors: VectorConnectionCache,
}
impl ModelCache {
    fn model(
        &mut self,
        directory: &Path,
        profile: &pkms_core::EmbeddingProfile,
        identity: &RequestIdentity,
    ) -> Result<&mut Embedder, &'static str> {
        let directory = directory
            .canonicalize()
            .map_err(|_| "embedding_model_unavailable")?;
        let key = serde_json::to_string(profile).map_err(|_| "embedding_profile_mismatch")?;
        if !self
            .model
            .as_ref()
            .is_some_and(|(path, cached, _)| path == &directory && cached == &key)
        {
            progress(identity, "loading_model", 0, None)?;
            // Free the prior model before allocating a different profile.
            self.model = None;
            let model = Embedder::load(&directory, profile).map_err(|error| error.0)?;
            self.model = Some((directory, key, model));
        }
        self.model
            .as_mut()
            .map(|(_, _, model)| model)
            .ok_or("embedding_model_unavailable")
    }
    fn tokenizer(&mut self, directory: &Path) -> Result<&Chunker, &'static str> {
        let directory = directory
            .canonicalize()
            .map_err(|_| "embedding_model_unavailable")?;
        if !self
            .tokenizer
            .as_ref()
            .is_some_and(|(path, _)| path == &directory)
        {
            self.tokenizer = None;
            let tokenizer = Chunker::load(&directory).map_err(|error| error.0)?;
            self.tokenizer = Some((directory, tokenizer));
        }
        self.tokenizer
            .as_ref()
            .map(|(_, tokenizer)| tokenizer)
            .ok_or("embedding_model_unavailable")
    }
}

fn emit(identity: &RequestIdentity, event: Event) -> Result<(), &'static str> {
    let event = WorkerEvent {
        protocol: PROTOCOL_VERSION,
        identity: identity.clone(),
        event,
    };
    let bytes = serde_json::to_vec(&event).map_err(|_| "inference_protocol_error")?;
    if bytes.len() > MAX_EVENT_BYTES {
        return Err("inference_event_limit");
    }
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&bytes)
        .and_then(|_| stdout.write_all(b"\n"))
        .and_then(|_| stdout.flush())
        .map_err(|_| "inference_output_unavailable")
}
fn progress(
    identity: &RequestIdentity,
    stage: &str,
    completed: u32,
    total: Option<u32>,
) -> Result<(), &'static str> {
    emit(
        identity,
        Event::Progress {
            stage: stage.into(),
            completed,
            total,
        },
    )
}
async fn run(
    request: &WorkerRequest,
    cache: &mut ModelCache,
) -> Result<WorkerResult, &'static str> {
    match &request.operation {
        Operation::ScoreExactQuotes {
            profile,
            query,
            inputs,
            limit,
        } => {
            pkms_inference::embedding::validate_exact_quote_request(query, inputs, *limit)
                .map_err(|error| error.0)?;
            progress(&request.identity, "tokenizing", 0, None)?;
            let embedder = cache.model(&request.model_directory, profile, &request.identity)?;
            let items = embedder
                .score_exact_quotes(profile, query, inputs, *limit, &mut |completed, total| {
                    progress(&request.identity, "embedding", completed, Some(total))
                        .map_err(pkms_inference::embedding::EmbeddingError)
                })
                .map_err(|error| error.0)?;
            Ok(WorkerResult::QuoteScores { items })
        }
        Operation::PrepareChunks { profile, input } => {
            progress(&request.identity, "tokenizing", 0, None)?;
            let chunker = cache.tokenizer(&request.model_directory)?;
            let plan = chunker.prepare(profile, input).map_err(|error| error.0)?;
            Ok(WorkerResult::Prepared { plan })
        }
        Operation::EmbedGeneration {
            profile,
            plan,
            title,
            generation_id,
        } => {
            if plan.profile_id != profile.id || plan.chunks.is_empty() {
                return Err("embedding_invalid_plan");
            }
            let embedder = cache.model(&request.model_directory, profile, &request.identity)?;
            let mut rows = Vec::with_capacity(plan.chunks.len());
            for batch in plan.chunks.chunks(8) {
                let inputs = batch
                    .iter()
                    .map(|chunk| {
                        format!("{}{}\n{}", profile.spec.passage_prefix, title, chunk.text)
                    })
                    .collect();
                let vectors = embedder.embed(inputs).map_err(|error| error.0)?;
                if vectors.len() != batch.len() {
                    return Err("embedding_invalid_vector");
                }
                for (chunk, vector) in batch.iter().zip(vectors) {
                    rows.push(VectorRow {
                        chunk_id: chunk.chunk_id.clone(),
                        object_id: plan.object_id.clone(),
                        content_revision: plan.content_revision.clone(),
                        vector,
                    });
                }
                progress(
                    &request.identity,
                    "embedding",
                    rows.len() as u32,
                    Some(plan.chunks.len() as u32),
                )?;
            }
            progress(&request.identity, "vector_commit", 0, None)?;
            let projection = cache
                .vectors
                .get(&request.vector_directory)
                .await
                .map_err(|_| "vector_open_failed")?;
            projection
                .publish(&profile.id, generation_id, DIMENSIONS, &rows)
                .await
                .map_err(|_| "vector_commit_failed")?;
            Ok(WorkerResult::Indexed {
                publish: pkms_core::IndexPublish {
                    generation_id: generation_id.clone(),
                    lease_token: request.identity.lease_token.clone(),
                    profile_id: profile.id.clone(),
                    chunk_count: rows.len() as u32,
                },
            })
        }
        Operation::SearchVectors {
            profile,
            generations,
            query,
            limit,
        } => {
            if generations.is_empty() {
                return Ok(WorkerResult::Candidates { items: Vec::new() });
            }
            let embedder = cache.model(&request.model_directory, profile, &request.identity)?;
            let vectors = embedder
                .embed(vec![format!("{}{query}", profile.spec.query_prefix)])
                .map_err(|error| error.0)?;
            let projection = cache
                .vectors
                .get(&request.vector_directory)
                .await
                .map_err(|_| "vector_open_failed")?;
            let expected: Vec<(String, u32)> = generations
                .iter()
                .map(|generation| (generation.generation_id.clone(), generation.chunk_count))
                .collect();
            projection
                .verify_generations(&profile.id, &expected)
                .await
                .map_err(|_| "projection_incomplete")?;
            let generation_ids: Vec<String> = generations
                .iter()
                .map(|generation| generation.generation_id.clone())
                .collect();
            let items = projection
                .search(&profile.id, &generation_ids, &vectors[0], *limit)
                .await
                .map_err(|_| "vector_query_failed")?;
            Ok(WorkerResult::Candidates { items })
        }
        Operation::PublishVectors {
            profile,
            plan,
            generation_id,
            vectors,
        } => {
            if !pkms_inference::remote_chunks::valid(profile)
                || plan.profile_id != profile.id
                || vectors.len() != plan.chunks.len()
                || vectors.len() > 100_000
            {
                return Err("embedding_profile_mismatch");
            }
            let rows = plan
                .chunks
                .iter()
                .zip(vectors)
                .map(|(chunk, vector)| VectorRow {
                    chunk_id: chunk.chunk_id.clone(),
                    object_id: plan.object_id.clone(),
                    content_revision: plan.content_revision.clone(),
                    vector: vector.clone(),
                })
                .collect::<Vec<_>>();
            let projection = cache
                .vectors
                .get(&request.vector_directory)
                .await
                .map_err(|_| "vector_open_failed")?;
            projection
                .publish(
                    &profile.id,
                    generation_id,
                    profile.spec.dimensions as usize,
                    &rows,
                )
                .await
                .map_err(|_| "vector_commit_failed")?;
            Ok(WorkerResult::Indexed {
                publish: pkms_core::IndexPublish {
                    generation_id: generation_id.clone(),
                    lease_token: request.identity.lease_token.clone(),
                    profile_id: profile.id.clone(),
                    chunk_count: rows.len() as u32,
                },
            })
        }
        Operation::SearchEmbedded {
            profile,
            generations,
            vector,
            limit,
        } => {
            if !pkms_inference::remote_chunks::valid(profile)
                || vector.len() != profile.spec.dimensions as usize
            {
                return Err("embedding_profile_mismatch");
            }
            let projection = cache
                .vectors
                .get(&request.vector_directory)
                .await
                .map_err(|_| "vector_open_failed")?;
            let expected = generations
                .iter()
                .map(|g| (g.generation_id.clone(), g.chunk_count))
                .collect::<Vec<_>>();
            projection
                .verify_generations(&profile.id, &expected)
                .await
                .map_err(|_| "projection_incomplete")?;
            let ids = generations
                .iter()
                .map(|g| g.generation_id.clone())
                .collect::<Vec<_>>();
            let items = projection
                .search(&profile.id, &ids, vector, *limit)
                .await
                .map_err(|_| "vector_query_failed")?;
            Ok(WorkerResult::Candidates { items })
        }
        Operation::Chat {
            endpoint,
            model,
            messages,
            num_ctx,
            num_predict,
            temperature,
            format,
        } => {
            let client = ollama::OllamaClient::new(
                endpoint,
                ollama::StreamLimits {
                    max_response_bytes: 1024 * 1024,
                    max_line_bytes: 64 * 1024,
                    ..Default::default()
                },
            )
            .map_err(|error| error.code())?;
            let input = ollama::ChatRequest {
                format: format.clone(),
                model: model.clone(),
                messages: messages
                    .iter()
                    .map(|message| ollama::ChatMessage {
                        role: match message.role {
                            ChatRole::System => ollama::ChatRole::System,
                            ChatRole::User => ollama::ChatRole::User,
                            ChatRole::Assistant => ollama::ChatRole::Assistant,
                        },
                        content: message.content.clone(),
                    })
                    .collect(),
                options: ollama::ChatOptions {
                    num_ctx: *num_ctx,
                    num_predict: *num_predict,
                    temperature: *temperature,
                },
            };
            let result = client
                .chat(
                    &input,
                    |text| {
                        // JSON escaping can expand one character sixfold; bounded chunks
                        // leave room for identity and NDJSON envelope.
                        let chars: Vec<char> = text.chars().collect();
                        for part in chars.chunks(4096) {
                            emit(
                                &request.identity,
                                Event::TextDelta {
                                    text: part.iter().collect(),
                                },
                            )
                            .map_err(|_| ollama::OllamaError::Canceled)?;
                        }
                        Ok(())
                    },
                    || false,
                )
                .await
                .map_err(|error| error.code())?;
            Ok(WorkerResult::Answer {
                done_reason: result.done_reason,
                prompt_eval_count: result.prompt_eval_count,
                eval_count: result.eval_count,
            })
        }
    }
}
#[derive(Default)]
struct WorkerRuntime(Option<tokio::runtime::Runtime>);
impl WorkerRuntime {
    fn get(&mut self) -> Result<&tokio::runtime::Runtime, &'static str> {
        if self.0.is_none() {
            self.0 = Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|_| "inference_runtime_unavailable")?,
            );
        }
        Ok(self.0.as_ref().expect("runtime initialized"))
    }
}
impl Drop for WorkerRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_timeout(std::time::Duration::from_millis(250));
        }
    }
}

fn execute(
    request: &WorkerRequest,
    cache: &mut ModelCache,
    runtime: &mut WorkerRuntime,
) -> Result<(), &'static str> {
    if request.protocol != PROTOCOL_VERSION
        || !request.result_path.is_absolute()
        || !request.model_directory.is_absolute()
        || !request.vector_directory.is_absolute()
        || request.identity.attempt == 0
        || [
            &request.identity.request_id,
            &request.identity.job_id,
            &request.identity.lease_token,
        ]
        .iter()
        .any(|value| value.is_empty() || value.len() > 256)
    {
        return Err("inference_invalid_request");
    }
    let result = runtime.get()?.block_on(run(request, cache));
    let result = serde_json::to_vec(&result?).map_err(|_| "inference_result_invalid")?;
    if result.len() > MAX_RESULT_BYTES {
        return Err("inference_result_limit");
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&request.result_path)
        .map_err(|_| "inference_result_unavailable")?;
    file.write_all(&result)
        .and_then(|_| file.sync_all())
        .map_err(|_| "inference_result_unavailable")?;
    emit(
        &request.identity,
        Event::Completed {
            result_path: request.result_path.clone(),
            sha256: format!("{:x}", Sha256::digest(&result)),
            bytes: result.len() as u64,
        },
    )
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--session") {
        session();
        return;
    }
    let mut bytes = Vec::new();
    if std::io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_REQUEST_BYTES
    {
        eprintln!("inference_invalid_request");
        std::process::exit(2);
    }
    let request: WorkerRequest = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(_) => {
            eprintln!("inference_invalid_request");
            std::process::exit(2);
        }
    };
    if let Err(code) = execute(
        &request,
        &mut ModelCache::default(),
        &mut WorkerRuntime::default(),
    ) {
        let _ = emit(&request.identity, Event::Failed { code: code.into() });
        std::process::exit(1);
    }
}

fn session() {
    let mut input = std::io::stdin().lock();
    let mut runtime = WorkerRuntime::default();
    let mut cache = ModelCache::default();
    loop {
        let mut bytes = Vec::new();
        let read = input
            .by_ref()
            .take(MAX_REQUEST_BYTES + 1)
            .read_until(b'\n', &mut bytes);
        if read.as_ref().is_ok_and(|size| *size == 0) {
            return;
        }
        if read.is_err() || bytes.len() as u64 > MAX_REQUEST_BYTES || bytes.last() != Some(&b'\n') {
            eprintln!("inference_invalid_frame");
            std::process::exit(2);
        }
        let request: WorkerRequest = match serde_json::from_slice(&bytes) {
            Ok(request) => request,
            Err(_) => {
                eprintln!("inference_invalid_request");
                std::process::exit(2);
            }
        };
        if let Err(code) = execute(&request, &mut cache, &mut runtime) {
            if matches!(
                code,
                "embedding_inference_failed"
                    | "embedding_invalid_vector"
                    | "embedding_invalid_vector_count"
            ) {
                cache.model = None;
            }
            if emit(&request.identity, Event::Failed { code: code.into() }).is_err() {
                return;
            }
        }
        // Wait for the next complete request. The owning supervisor controls
        // idle lifetime; EOF exits, and kill/wait interrupts active inference.
    }
}

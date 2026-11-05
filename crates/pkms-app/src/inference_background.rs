use crate::{
    AppError, AppService, InferenceConfig, InferenceConnection, Result, WorkspaceIdentity,
};
use pkms_core::{AnswerRecord, RebuildJob, RebuildStatus};
use pkms_inference::protocol::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, atomic::Ordering},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Schedule {
    running: bool,
    requested: bool,
}
pub type InferenceChanged<'a> =
    dyn FnMut(&WorkspaceIdentity, Option<&AnswerRecord>, Option<&RebuildJob>) + 'a;
pub struct InferenceJobsWorker {
    connection: InferenceConnection,
    schedule: Arc<Mutex<Schedule>>,
    released: bool,
}
impl Drop for InferenceJobsWorker {
    fn drop(&mut self) {
        if !self.released {
            self.schedule
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .running = false;
        }
    }
}
impl AppService {
    pub fn inference_jobs_worker(&self) -> Result<Option<InferenceJobsWorker>> {
        let Some(active) = self.active_workspace() else {
            return Ok(None);
        };
        let connection = self.inference_connection(&active.identity)?;
        static SCHEDULES: OnceLock<Mutex<HashMap<String, Arc<Mutex<Schedule>>>>> = OnceLock::new();
        let schedule = {
            let mut map = SCHEDULES
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| AppError::AppBusy)?;
            map.entry(connection.storage_identity.clone())
                .or_default()
                .clone()
        };
        {
            let mut state = schedule.lock().map_err(|_| AppError::AppBusy)?;
            state.requested = true;
            if state.running {
                return Ok(None);
            }
            state.running = true;
        }
        Ok(Some(InferenceJobsWorker {
            connection,
            schedule,
            released: false,
        }))
    }
}
impl InferenceJobsWorker {
    pub fn run(self, config: InferenceConfig, changed: &mut InferenceChanged<'_>) -> Result<()> {
        self.run_with_progress(config, changed, &mut |_, _| {})
    }
    pub fn run_with_progress(
        self,
        config: InferenceConfig,
        changed: &mut InferenceChanged<'_>,
        progress: &mut dyn FnMut(&WorkspaceIdentity, &crate::AgentProgress),
    ) -> Result<()> {
        self.run_with_chat(config, changed, progress, &mut |_, _| {})
    }
    pub fn run_with_chat(
        self,
        config: InferenceConfig,
        changed: &mut InferenceChanged<'_>,
        progress: &mut dyn FnMut(&WorkspaceIdentity, &crate::AgentProgress),
        chat_changed: &mut dyn FnMut(&WorkspaceIdentity, &pkms_core::ChatRun),
    ) -> Result<()> {
        self.run_with_chat_progress(config, changed, progress, chat_changed, &mut |_, _| {})
    }
    pub fn run_with_chat_progress(
        mut self,
        config: InferenceConfig,
        changed: &mut InferenceChanged<'_>,
        progress: &mut dyn FnMut(&WorkspaceIdentity, &crate::AgentProgress),
        chat_changed: &mut dyn FnMut(&WorkspaceIdentity, &pkms_core::ChatRun),
        chat_progress: &mut dyn FnMut(&WorkspaceIdentity, &crate::ChatProgress),
    ) -> Result<()> {
        self.connection.library.recover_answers_exclusive()?;
        self.connection.library.recover_chat_runs_exclusive()?;
        self.connection.library.recover_index_rebuilds_exclusive()?;
        self.connection
            .library
            .recover_index_generations_exclusive()?;
        loop {
            self.schedule
                .lock()
                .map_err(|_| AppError::AppBusy)?
                .requested = false;
            let mut automatic_started = false;
            loop {
                if config.shutdown.load(Ordering::Acquire) {
                    return Ok(());
                }
                let answers = self.connection.library.pending_answer_ids(10)?;
                let chats = self.connection.library.pending_chat_run_ids(10)?;
                let rebuilds = self.connection.library.pending_index_rebuild_ids(10)?;
                if answers.is_empty() && chats.is_empty() && rebuilds.is_empty() {
                    if !automatic_started {
                        automatic_started = true;
                        let objects = self.connection.library.dirty_index_object_ids(32)?;
                        if !objects.is_empty()
                            && self
                                .connection
                                .library
                                .active_embedding_profile()?
                                .as_ref()
                                .is_some_and(crate::inference_embeddings::supported)
                        {
                            let profile = self
                                .connection
                                .library
                                .active_embedding_profile()?
                                .ok_or(AppError::InvalidInput("embedding_unconfigured"))?;
                            let job = self
                                .connection
                                .library
                                .accept_index_rebuild(
                                    &uuid::Uuid::new_v4().to_string(),
                                    pkms_core::RebuildRequest {
                                        profile_id: profile.id,
                                        scope: pkms_core::SearchScope {
                                            object_ids: objects.clone(),
                                            ..Default::default()
                                        },
                                    },
                                )?
                                .job;
                            let identity = self.connection.identity.clone();
                            self.connection.run_rebuild(&config, &job.id, &mut |job| {
                                changed(&identity, None, Some(job))
                            })?;
                            let outcome = self.connection.library.index_rebuild_job(&job.id)?;
                            if outcome.status == RebuildStatus::Failed {
                                self.connection.library.defer_dirty_index_rebuild(
                                    &job.id,
                                    outcome.error_code.as_deref().unwrap_or("inference_failed"),
                                )?;
                            }
                            continue;
                        }
                    }
                    break;
                }
                for id in answers {
                    if config.shutdown.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let identity = self.connection.identity.clone();
                    self.connection.run_answer_with_progress(
                        &config,
                        &id,
                        &mut |answer| changed(&identity, Some(answer), None),
                        &mut |stage| progress(&identity, stage),
                    )?;
                }
                for id in chats {
                    if config.shutdown.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let identity = self.connection.identity.clone();
                    self.connection.run_chat(
                        &config,
                        &id,
                        &mut |run| chat_changed(&identity, run),
                        &mut |stage| chat_progress(&identity, stage),
                    )?;
                }
                for id in rebuilds {
                    if config.shutdown.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let identity = self.connection.identity.clone();
                    self.connection.run_rebuild(&config, &id, &mut |job| {
                        changed(&identity, None, Some(job))
                    })?;
                }
            }
            let remote_enabled = self
                .connection
                .library
                .active_embedding_profile()?
                .as_ref()
                .is_some_and(crate::inference_embeddings::supported);
            let remaining = remote_enabled
                && !self
                    .connection
                    .library
                    .dirty_index_object_ids(1)?
                    .is_empty();
            let mut state = self.schedule.lock().map_err(|_| AppError::AppBusy)?;
            if remaining {
                state.requested = true;
            }
            if state.requested {
                continue;
            }
            state.running = false;
            self.released = true;
            return Ok(());
        }
    }
}
struct PreparedIndex {
    title: String,
    plan: pkms_core::IndexPlan,
}

impl InferenceConnection {
    fn run_rebuild(
        &mut self,
        config: &InferenceConfig,
        id: &str,
        changed: &mut dyn FnMut(&RebuildJob),
    ) -> Result<()> {
        let Some(lease) = self.library.claim_index_rebuild(id)? else {
            return Ok(());
        };
        changed(&lease.job);
        let result = self.rebuild_inner(config, &lease, changed);
        if let Err(error) = result
            && self.library.index_rebuild_job(id)?.status == RebuildStatus::Running
        {
            let failed = self.library.fail_index_rebuild(
                id,
                &lease.lease_token,
                crate::inference_queries::error_code(&error),
            )?;
            changed(&failed);
        }
        Ok(())
    }

    // Preserve manifest order. Empty/unavailable entries after a nonempty batch
    // are handled on the next iteration, after all preceding entries publish.
    fn prepare_rebuild_batch(
        &mut self,
        profile: &pkms_core::EmbeddingProfile,
        lease: &pkms_core::RebuildLease,
        objects: &[String],
        changed: &mut dyn FnMut(&RebuildJob),
    ) -> Result<Vec<PreparedIndex>> {
        let mut batch = Vec::new();
        let mut chunks = 0;
        for object in objects {
            let input = match self.library.index_input(object) {
                Ok(input) => input,
                Err(pkms_core::Error::NotFound | pkms_core::Error::Trashed) => {
                    if !batch.is_empty() {
                        break;
                    }
                    changed(&self.library.checkpoint_index_rebuild(
                        &lease.job.id,
                        &lease.lease_token,
                        object,
                        None,
                        Some("object_unavailable"),
                    )?);
                    continue;
                }
                Err(error) => {
                    if !batch.is_empty() {
                        break;
                    }
                    return Err(error.into());
                }
            };
            let title = input.title.clone();
            let plan = match pkms_inference::remote_chunks::prepare(profile, input.clone()) {
                Ok(plan) => plan,
                Err(code) => {
                    // The durable retry policy attributes failure to the first
                    // unprocessed object. Publish preceding valid plans first.
                    if !batch.is_empty() {
                        break;
                    }
                    return Err(AppError::InvalidInput(code));
                }
            };
            if plan.chunks.is_empty() {
                if !batch.is_empty() {
                    break;
                }
                self.library.clear_dirty_index_input(&profile.id, &input)?;
                changed(&self.library.checkpoint_index_rebuild(
                    &lease.job.id,
                    &lease.lease_token,
                    object,
                    None,
                    Some("empty_content"),
                )?);
                continue;
            }
            // A large single object uses the transport's existing bounded batches.
            // Multiple objects together never exceed one 32-input request.
            if !batch.is_empty()
                && chunks + plan.chunks.len() > pkms_inference::remote_embeddings::MAX_BATCH
            {
                break;
            }
            chunks += plan.chunks.len();
            batch.push(PreparedIndex { title, plan });
            if chunks >= pkms_inference::remote_embeddings::MAX_BATCH {
                break;
            }
        }
        Ok(batch)
    }

    fn rebuild_inner(
        &mut self,
        config: &InferenceConfig,
        lease: &pkms_core::RebuildLease,
        changed: &mut dyn FnMut(&RebuildJob),
    ) -> Result<()> {
        let profile = self
            .library
            .active_embedding_profile()?
            .ok_or(AppError::InvalidInput("embedding_unconfigured"))?;
        if !crate::inference_embeddings::supported(&profile) {
            return Err(AppError::InvalidInput("embedding_remote_required"));
        }
        if profile.id != lease.job.request.profile_id {
            return Err(AppError::InvalidInput("embedding_profile_changed"));
        }
        if crate::inference_embeddings::local(&profile) {
            return self.rebuild_local(config, &profile, lease, changed);
        }
        loop {
            if config.shutdown.load(Ordering::Acquire) {
                return Err(AppError::InvalidInput("worker_interrupted"));
            }
            self.control
                .heartbeat_index_rebuild(&lease.job.id, &lease.lease_token)?;
            let objects = self.library.index_rebuild_next_objects(&lease.job.id, 32)?;
            if objects.is_empty() {
                changed(
                    &self
                        .library
                        .finish_index_rebuild(&lease.job.id, &lease.lease_token)?,
                );
                return Ok(());
            }
            let batch = self.prepare_rebuild_batch(&profile, lease, &objects, changed)?;
            if !batch.is_empty() {
                self.publish_rebuild_batch(config, &profile, lease, batch, changed)?;
            }
        }
    }

    /// Local E5 runs in the resident inference worker. It prepares chunks with
    /// the pinned tokenizer, embeds them, and publishes the projection in one
    /// operation; the app then commits the same generation manifest used by the
    /// remote path.
    fn rebuild_local(
        &mut self,
        config: &InferenceConfig,
        profile: &pkms_core::EmbeddingProfile,
        lease: &pkms_core::RebuildLease,
        changed: &mut dyn FnMut(&RebuildJob),
    ) -> Result<()> {
        loop {
            if config.shutdown.load(Ordering::Acquire) {
                return Err(AppError::InvalidInput("worker_interrupted"));
            }
            self.control
                .heartbeat_index_rebuild(&lease.job.id, &lease.lease_token)?;
            let objects = self.library.index_rebuild_next_objects(&lease.job.id, 8)?;
            if objects.is_empty() {
                changed(
                    &self
                        .library
                        .finish_index_rebuild(&lease.job.id, &lease.lease_token)?,
                );
                return Ok(());
            }
            for object in objects {
                if config.shutdown.load(Ordering::Acquire) {
                    return Err(AppError::InvalidInput("worker_interrupted"));
                }
                self.control
                    .heartbeat_index_rebuild(&lease.job.id, &lease.lease_token)?;
                let input = match self.library.index_input(&object) {
                    Ok(input) => input,
                    Err(pkms_core::Error::NotFound | pkms_core::Error::Trashed) => {
                        changed(&self.library.checkpoint_index_rebuild(
                            &lease.job.id,
                            &lease.lease_token,
                            &object,
                            None,
                            Some("object_unavailable"),
                        )?);
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                };
                let title = input.title.clone();
                let temp = tempfile::Builder::new()
                    .prefix("pkms-local-index-")
                    .tempdir()?;
                let prepare = crate::inference_queries::request(
                    config,
                    &self.identity,
                    Operation::PrepareChunks {
                        profile: profile.clone(),
                        input: input.clone(),
                    },
                    RequestIdentity {
                        request_id: uuid::Uuid::new_v4().to_string(),
                        job_id: lease.job.id.clone(),
                        attempt: 1,
                        lease_token: lease.lease_token.clone(),
                    },
                    &temp.path().join("prepare.json"),
                );
                let plan = match crate::inference_process::run(
                    config,
                    prepare,
                    Duration::from_secs(120),
                    &mut |_| Ok(()),
                )? {
                    WorkerResult::Prepared { plan } => plan,
                    _ => return Err(AppError::InvalidInput("inference_result_kind")),
                };
                if plan.chunks.is_empty() {
                    self.library.clear_dirty_index_input(&profile.id, &input)?;
                    changed(&self.library.checkpoint_index_rebuild(
                        &lease.job.id,
                        &lease.lease_token,
                        &object,
                        None,
                        Some("empty_content"),
                    )?);
                    continue;
                }
                let generation = self
                    .library
                    .begin_index_generation(&uuid::Uuid::new_v4().to_string(), plan.clone())?;
                let embed = crate::inference_queries::request(
                    config,
                    &self.identity,
                    Operation::EmbedGeneration {
                        profile: profile.clone(),
                        plan,
                        title,
                        generation_id: generation.generation.id.clone(),
                    },
                    RequestIdentity {
                        request_id: uuid::Uuid::new_v4().to_string(),
                        job_id: generation.generation.id.clone(),
                        attempt: 1,
                        lease_token: generation.lease_token.clone(),
                    },
                    &temp.path().join("embed.json"),
                );
                let publish = match crate::inference_process::run(
                    config,
                    embed,
                    Duration::from_secs(u64::from(pkms_core::embedding_timeout_seconds(false))),
                    &mut |_| Ok(()),
                )? {
                    WorkerResult::Indexed { publish } => publish,
                    _ => return Err(AppError::InvalidInput("inference_result_kind")),
                };
                if publish.generation_id != generation.generation.id
                    || publish.lease_token != generation.lease_token
                    || publish.profile_id != profile.id
                    || publish.chunk_count != generation.generation.chunk_count
                {
                    return Err(AppError::InvalidInput("inference_manifest_mismatch"));
                }
                let job = self.library.publish_rebuild_generation(
                    &lease.job.id,
                    &lease.lease_token,
                    publish,
                )?;
                changed(&job);
            }
        }
    }

    fn publish_rebuild_batch(
        &mut self,
        config: &InferenceConfig,
        profile: &pkms_core::EmbeddingProfile,
        lease: &pkms_core::RebuildLease,
        batch: Vec<PreparedIndex>,
        changed: &mut dyn FnMut(&RebuildJob),
    ) -> Result<()> {
        let mut generations = Vec::new();
        let completed = std::cell::Cell::new(0usize);
        let result = (|| {
            for object in &batch {
                generations.push(self.library.begin_index_generation(
                    &uuid::Uuid::new_v4().to_string(),
                    object.plan.clone(),
                )?);
            }
            let mut renewed = Instant::now();
            let mut checkpoint = || {
                if config.shutdown.load(Ordering::Acquire) {
                    return Err(AppError::InvalidInput("worker_interrupted"));
                }
                if self.control.index_rebuild_job(&lease.job.id)?.status != RebuildStatus::Running {
                    return Err(AppError::InvalidInput("canceled"));
                }
                if renewed.elapsed() >= Duration::from_secs(20) {
                    self.control
                        .heartbeat_index_rebuild(&lease.job.id, &lease.lease_token)?;
                    for generation in generations.iter().skip(completed.get()) {
                        self.control.heartbeat_index_generation(
                            &generation.generation.id,
                            &generation.lease_token,
                        )?;
                    }
                    renewed = Instant::now();
                }
                Ok(())
            };
            checkpoint()?;
            if profile.spec.provider.is_some() {
                crate::privacy::require_remote_consent(&self.library)?;
            }
            let texts = batch
                .iter()
                .flat_map(|object| {
                    object
                        .plan
                        .chunks
                        .iter()
                        .map(|c| format!("{}\n{}", object.title, c.text))
                })
                .collect::<Vec<_>>();
            let vectors = crate::inference_embeddings::embed(
                &self.identity,
                profile,
                crate::inference_embeddings::Kind::Document,
                &texts,
                config,
                Duration::from_secs(u64::from(pkms_core::embedding_timeout_seconds(
                    profile.spec.provider.is_some(),
                ))),
                &mut checkpoint,
            )?;
            // Index builds dominate embedding usage; record the estimate here so
            // the statistics panel covers background work, not only retrieval.
            crate::inference_queries::record_embedding_texts(&mut self.library, &texts);
            if vectors.len() != texts.len() {
                return Err(AppError::InvalidInput("embedding_invalid_vector_count"));
            }
            let mut vectors = vectors.into_iter();
            let temp = tempfile::Builder::new().prefix("pkms-index-").tempdir()?;
            for (i, (object, generation)) in batch.into_iter().zip(&generations).enumerate() {
                checkpoint()?;
                let chunk_vectors = vectors.by_ref().take(object.plan.chunks.len()).collect();
                let request = crate::inference_queries::request(
                    config,
                    &self.identity,
                    Operation::PublishVectors {
                        profile: profile.clone(),
                        plan: object.plan,
                        generation_id: generation.generation.id.clone(),
                        vectors: chunk_vectors,
                    },
                    RequestIdentity {
                        request_id: uuid::Uuid::new_v4().to_string(),
                        job_id: generation.generation.id.clone(),
                        attempt: 1,
                        lease_token: generation.lease_token.clone(),
                    },
                    &temp.path().join(format!("publish-{i}.json")),
                );
                let publish = match crate::inference_process::run(
                    config,
                    request,
                    Duration::from_secs(120),
                    &mut |_| checkpoint(),
                )? {
                    WorkerResult::Indexed { publish } => publish,
                    _ => return Err(AppError::InvalidInput("inference_result_kind")),
                };
                if publish.generation_id != generation.generation.id
                    || publish.lease_token != generation.lease_token
                    || publish.profile_id != profile.id
                    || publish.chunk_count != generation.generation.chunk_count
                {
                    return Err(AppError::InvalidInput("inference_manifest_mismatch"));
                }
                let job = self.library.publish_rebuild_generation(
                    &lease.job.id,
                    &lease.lease_token,
                    publish,
                )?;
                completed.set(i + 1);
                changed(&job);
            }
            Ok(())
        })();
        if let Err(error) = result {
            // Earlier publications stay committed. All remaining leases must be
            // terminal, including those after a failed request or canceled job.
            let mut cleanup_error = None;
            for generation in generations.iter().skip(completed.get()) {
                if let Err(failed) = self.library.fail_index_generation(
                    &generation.generation.id,
                    &generation.lease_token,
                    crate::inference_queries::error_code(&error),
                ) && cleanup_error.is_none()
                {
                    cleanup_error = Some(AppError::from(failed));
                }
            }
            return Err(cleanup_error.unwrap_or(error));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "inference_batch_tests.rs"]
mod batch_tests;

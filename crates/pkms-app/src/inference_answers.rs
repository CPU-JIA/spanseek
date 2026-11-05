use crate::{AppError, InferenceConfig, InferenceConnection, Result};
use pkms_core::{AnswerRecord, AnswerStatus, CompleteAnswer, GenerationProfile};
use pkms_inference::protocol::*;
use std::time::{Duration, Instant};

pub(crate) fn messages(answer: &AnswerRecord) -> Vec<ChatMessage> {
    let mut evidence = String::new();
    for item in &answer.evidence {
        // Small-to-big: the model reads the frozen parent context when the hit
        // itself is only a narrow chunk; the citation still points at the hit.
        let text = item.context.as_deref().unwrap_or(&item.evidence.text);
        evidence.push_str(&format!(
            "\n[{}] {}\n{}\n",
            item.citation_id, item.evidence.title, text
        ));
    }
    vec![
        ChatMessage {
            role: ChatRole::System,
            content: include_str!("inference_answer_prompt.txt").trim().into(),
        },
        ChatMessage {
            role: ChatRole::User,
            content: format!(
                "Question:\n{}\n\nEvidence:{}",
                answer.request.search.query, evidence
            ),
        },
    ]
}
/// Local service must actually hold the exact requested digest; an Ollama cloud alias is not a local model.
pub fn verify_generation_model(
    profile: &GenerationProfile,
    shutdown: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    crate::inference_model_policy::validate_profile(profile)?;
    let service = crate::inference_models::ModelService::new(&profile.endpoint)?;
    crate::inference_models::run_model_request(shutdown, async {
        let body = service.get("api/tags").await?;
        let present = body
            .get("models")
            .and_then(|v| v.as_array())
            .is_some_and(|models| {
                models.iter().any(|model| {
                    model.get("name").and_then(|v| v.as_str()) == Some(profile.model.as_str())
                        && model
                            .get("digest")
                            .and_then(|v| v.as_str())
                            .is_some_and(|digest| {
                                digest.strip_prefix("sha256:").unwrap_or(digest)
                                    == profile
                                        .model_digest
                                        .strip_prefix("sha256:")
                                        .unwrap_or(&profile.model_digest)
                            })
                        && model
                            .get("size")
                            .and_then(|v| v.as_u64())
                            .is_some_and(|size| size > 0)
                        && model.get("remote_model").is_none()
                        && model.get("remote_host").is_none()
                })
            });
        if !present {
            return Err(AppError::InvalidInput("model_unavailable"));
        }
        let show = service.show(&profile.model).await?;
        let runtime = service.get("api/version").await?;
        crate::inference_model_policy::validate_show(
            profile,
            &show,
            crate::inference_models::runtime_version(&runtime)?,
        )
    })
}

impl InferenceConnection {
    pub(crate) fn run_answer_with_progress(
        &mut self,
        config: &InferenceConfig,
        id: &str,
        changed: &mut dyn FnMut(&AnswerRecord),
        progress: &mut dyn FnMut(&crate::inference_agent::AgentProgress),
    ) -> Result<()> {
        let Some(lease) = self.library.claim_answer(id)? else {
            return Ok(());
        };
        changed(&lease.answer);
        let result = self.answer_inner(config, &lease, changed, progress);
        if let Err(error) = result {
            let current = self.library.answer(id)?;
            if current.status == AnswerStatus::Running {
                let code = crate::inference_queries::error_code(&error);
                let status = if matches!(
                    code,
                    "inference_timeout"
                        | "model_total_timeout"
                        | "model_first_token_timeout"
                        | "model_idle_timeout"
                        | "agent_total_timeout"
                ) {
                    AnswerStatus::TimedOut
                } else {
                    AnswerStatus::Failed
                };
                let failed = self
                    .library
                    .fail_answer(id, &lease.lease_token, status, code)?;
                changed(&failed);
            }
        }
        Ok(())
    }
    fn answer_inner(
        &mut self,
        config: &InferenceConfig,
        lease: &pkms_core::AnswerLease,
        changed: &mut dyn FnMut(&AnswerRecord),
        progress: &mut dyn FnMut(&crate::inference_agent::AgentProgress),
    ) -> Result<()> {
        let id = &lease.answer.id;
        let token = &lease.lease_token;
        let mut renewed = Instant::now();
        let mut check = || {
            if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                return Err(AppError::InvalidInput("worker_interrupted"));
            }
            self.control.check_answer_lease(id, token)?;
            if renewed.elapsed() >= Duration::from_secs(20) {
                self.control.heartbeat_answer(id, token)?;
                renewed = Instant::now();
            }
            Ok(())
        };
        if lease.answer.request.agent.is_some() {
            return crate::inference_agent::run(
                &mut self.library,
                &self.identity,
                config,
                lease,
                &mut check,
                changed,
                progress,
            );
        }
        let binding = RequestIdentity {
            request_id: uuid::Uuid::new_v4().to_string(),
            job_id: id.clone(),
            attempt: 1,
            lease_token: token.clone(),
        };
        let retrieval_started = Instant::now();
        // With the semantic capability disabled or consent withdrawn, evidence gathering stays local.
        let settings = self.library.retrieval_settings()?.params;
        let embedding = self.library.active_embedding_profile()?;
        let remote = embedding
            .as_ref()
            .is_some_and(|profile| profile.spec.provider.is_some());
        let candidates = if settings.semantic_enabled && (!remote || settings.remote_consent) {
            crate::inference_queries::candidates(
                &self.library,
                &self.identity,
                config,
                &lease.answer.request.search,
                binding.clone(),
                &mut check,
            )?
        } else {
            crate::inference_queries::QueryCandidates::default()
        };
        // Retrieval latency and embedding usage are best-effort telemetry and
        // never change what the answer pipeline returns.
        let _ = self.library.record_retrieval(
            lease.answer.request.search.mode,
            crate::inference_queries::elapsed_ms(retrieval_started),
            candidates.vectors.len() as u64,
        );
        crate::inference_queries::record_embedding_usage(
            &mut self.library,
            candidates.embedding_calls,
            candidates.estimated_embedding_tokens,
        );
        let answer = self.library.prepare_answer_evidence_with_annotations(
            id,
            token,
            candidates.vectors,
            candidates.annotations,
        )?;
        changed(&answer);
        if answer.status != AnswerStatus::Running {
            return Ok(());
        }
        let profile = answer
            .request
            .model
            .as_ref()
            .ok_or(AppError::InvalidInput("model_unconfigured"))?;
        crate::privacy::require_generation_consent(&self.library, profile)?;
        let messages = messages(&answer);
        if pkms_inference::profile::chat_context_bound(
            messages.iter().map(|m| m.content.len()).sum(),
            messages.len(),
            answer.request.output_tokens(),
        ) > profile.context_tokens as usize
        {
            return Err(AppError::InvalidInput("model_context_limit"));
        }
        check()?;
        // The library setting wins; otherwise the provider decides, because a
        // model running on this machine is allowed to take longer than a remote
        // one. Both ends are enforced when the setting is saved.
        let answer_timeout = self
            .library
            .retrieval_settings()?
            .params
            .answer_timeout_seconds
            .unwrap_or_else(|| {
                pkms_core::default_timeout_seconds(
                    &profile.provider,
                    pkms_core::GenerationPurpose::Answer,
                )
            });
        let temp = tempfile::Builder::new().prefix("pkms-answer-").tempdir()?;
        let allowed: Vec<String> = answer
            .evidence
            .iter()
            .map(|item| item.citation_id.clone())
            .collect();
        let started = std::time::Instant::now();
        let mut first_token_ms = None;
        let (output, usage) = if profile.provider != "ollama" {
            {
                let (text, usage, first) = crate::inference_remote::answer(
                    &self.identity,
                    profile,
                    messages,
                    allowed.clone(),
                    answer.request.output_tokens(),
                    Duration::from_secs(u64::from(answer_timeout)),
                    &mut || {
                        if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                            return Err(AppError::InvalidInput("worker_interrupted"));
                        }
                        check()
                    },
                )?;
                first_token_ms = first.map(pkms_core::duration_ms);
                (text, usage)
            }
        } else {
            verify_generation_model(profile, &config.shutdown)?;
            let operation = Operation::Chat {
                endpoint: profile.endpoint.clone(),
                model: profile.model.clone(),
                messages,
                num_ctx: profile.context_tokens,
                num_predict: answer.request.output_tokens(),
                temperature: profile.temperature as f32,
                format: Some(ChatFormat::EvidenceAnswer {
                    citation_ids: allowed.clone(),
                }),
            };
            let mut output = String::new();
            let mut first_token_seen = false;
            let result = crate::inference_process::run(
                config,
                crate::inference_queries::request(
                    config,
                    &self.identity,
                    operation,
                    binding,
                    &temp.path().join("result.json"),
                ),
                Duration::from_secs(u64::from(answer_timeout)),
                &mut |event| {
                    check()?;
                    if let Some(Event::TextDelta { text }) = event {
                        if output.len() + text.len() > 1024 * 1024 || text.len() > 8192 {
                            return Err(AppError::InvalidInput("answer_output_limit"));
                        }
                        output.push_str(text);
                        if !first_token_seen && !text.is_empty() {
                            first_token_seen = true;
                            first_token_ms = Some(pkms_core::elapsed_ms(started));
                            // Report actual model progress without exposing the wire JSON.
                            changed(&answer);
                        }
                    }
                    Ok(())
                },
            )?;
            match result {
                WorkerResult::Answer {
                    done_reason,
                    prompt_eval_count,
                    eval_count,
                } if done_reason.as_deref() == Some("stop") => (
                    output,
                    pkms_inference::remote::Usage {
                        input_tokens: prompt_eval_count,
                        output_tokens: eval_count,
                        reasoning_tokens: None,
                    },
                ),
                WorkerResult::Answer { .. } => {
                    return Err(AppError::InvalidInput("model_output_truncated"));
                }
                _ => return Err(AppError::InvalidInput("inference_result_kind")),
            }
        };
        // Provider usage is recorded before decoding: a rejected or truncated
        // output still consumed tokens and must stay visible in the panel.
        let _ = self.library.record_model_usage(
            pkms_core::UsagePurpose::Answer,
            &profile.provider,
            &profile.model,
            pkms_core::ModelUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                estimated: false,
                latency: Some(pkms_core::ModelCallLatency {
                    duration_ms: pkms_core::elapsed_ms(started),
                    first_token_ms,
                }),
            },
        );
        // Buffer wire tokens until the complete structure is validated. A single
        // atomic completion exposes human claims, never JSON or simulated streaming.
        check()?;
        let result = match crate::inference_answer_format::decode(&output, &allowed)? {
            crate::inference_answer_format::Answer::InsufficientEvidence => {
                self.library.complete_answer_insufficient_evidence(
                    id,
                    token,
                    pkms_core::INSUFFICIENT_EVIDENCE_MARKER,
                )?
            }
            crate::inference_answer_format::Answer::Supported { text, citations } => {
                self.library.complete_answer(CompleteAnswer {
                    answer_id: id.clone(),
                    lease_token: token.clone(),
                    cited_ids: citations,
                    text,
                })?
            }
        };
        changed(&result);
        Ok(())
    }
}

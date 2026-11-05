use crate::{AppError, AppService, InferenceConfig, Result, WorkspaceIdentity, storage_identity};
use pkms_core::{
    AnnotationVectorCandidate, Library, SearchMode, SearchQuery, SearchResults, VectorCandidate,
    WorkspaceLease,
};
use pkms_inference::protocol::*;
use std::{
    fs::File,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct InferenceConnection {
    pub(crate) library: Library,
    pub(crate) control: Library,
    pub(crate) identity: WorkspaceIdentity,
    pub(crate) storage_identity: String,
    _workspace_lease: Arc<WorkspaceLease>,
    _app_lease: Arc<File>,
}
impl AppService {
    pub fn inference_connection(
        &self,
        identity: &WorkspaceIdentity,
    ) -> Result<InferenceConnection> {
        self.library(identity)?;
        let active = self.active.as_ref().ok_or(AppError::InstanceMismatch)?;
        if storage_identity(Path::new(&active.entry.path))? != active.storage_identity {
            return Err(AppError::InstanceMismatch);
        }
        let library = Library::open_existing(&active.entry.path)?;
        if library.info()?.workspace_id != identity.workspace_id {
            return Err(AppError::InstanceMismatch);
        }
        let control = Library::open_existing(&active.entry.path)?;
        Ok(InferenceConnection {
            library,
            control,
            identity: identity.clone(),
            storage_identity: active.storage_identity.clone(),
            _workspace_lease: active._lease.clone(),
            _app_lease: self._lock.clone(),
        })
    }
}
pub(crate) fn request(
    config: &InferenceConfig,
    identity: &WorkspaceIdentity,
    operation: Operation,
    lease: RequestIdentity,
    result: &Path,
) -> WorkerRequest {
    WorkerRequest {
        protocol: PROTOCOL_VERSION,
        identity: lease,
        model_directory: config.model_directory.clone(),
        vector_directory: config
            .vector_root
            .join(&identity.local_instance_id)
            .join(&identity.workspace_id),
        result_path: result.to_owned(),
        operation,
    }
}
#[derive(Default)]
pub(crate) struct QueryCandidates {
    pub vectors: Vec<VectorCandidate>,
    pub annotations: Vec<AnnotationVectorCandidate>,
    /// Embedding calls performed for this query, plus a documented estimate used
    /// only when the provider reports no usage.
    pub embedding_calls: u64,
    pub estimated_embedding_tokens: u64,
}
/// Rough token estimate for providers that report no embedding usage: Latin text
/// averages about four characters per token, CJK about one character per token.
pub(crate) fn estimate_tokens(text: &str) -> u64 {
    let mut cjk = 0u64;
    let mut other = 0u64;
    for ch in text.chars() {
        if matches!(
            ch as u32,
            0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0xac00..=0xd7af
        ) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + other.div_ceil(4)
}

pub(crate) fn candidates(
    library: &Library,
    identity: &WorkspaceIdentity,
    config: &InferenceConfig,
    query: &SearchQuery,
    lease: RequestIdentity,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<QueryCandidates> {
    candidates_with_timeout(
        library,
        identity,
        config,
        query,
        lease,
        Duration::from_secs(120),
        check,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn candidates_with_timeout(
    library: &Library,
    identity: &WorkspaceIdentity,
    config: &InferenceConfig,
    query: &SearchQuery,
    lease: RequestIdentity,
    timeout: Duration,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<QueryCandidates> {
    let started = Instant::now();
    let remaining = || {
        timeout
            .checked_sub(started.elapsed())
            .filter(|duration| !duration.is_zero())
            .ok_or(AppError::InvalidInput("inference_timeout"))
    };
    query.validate()?;
    let mut output = QueryCandidates::default();
    check()?;
    remaining()?;
    if query.mode == SearchMode::Lexical {
        return Ok(output);
    }
    let Some(profile) = library.active_embedding_profile()? else {
        return Ok(output);
    };
    if crate::inference_embeddings::local(&profile) {
        return local_candidates(library, identity, config, query, lease, timeout, check);
    }
    if !crate::inference_embeddings::supported(&profile) {
        return Err(AppError::InvalidInput("embedding_remote_required"));
    }
    let annotations = library.annotation_search_inputs(&query.scope)?;
    let active = library.active_index_generations_in_scope(&query.scope)?;
    let limit = (query.limit as usize * 8).clamp(32, 800);
    let query_vector = if !active.is_empty() || !annotations.is_empty() {
        crate::inference_embeddings::embed(
            identity,
            &profile,
            crate::inference_embeddings::Kind::Query,
            std::slice::from_ref(&query.query),
            config,
            remaining()?,
            check,
        )?
        .pop()
        .ok_or(AppError::InvalidInput("embedding_invalid_vector_count"))?
    } else {
        Vec::new()
    };
    if !query_vector.is_empty() {
        output.embedding_calls += 1;
        output.estimated_embedding_tokens += estimate_tokens(&query.query);
    }
    if !active.is_empty() {
        let generations = active
            .into_iter()
            .filter(|generation| generation.profile_id == profile.id)
            .map(|generation| GenerationExpectation {
                generation_id: generation.id,
                chunk_count: generation.chunk_count,
            })
            .collect();
        let temp = tempfile::Builder::new()
            .prefix("pkms-vector-query-")
            .tempdir()?;
        let operation = Operation::SearchEmbedded {
            profile: profile.clone(),
            generations,
            vector: query_vector.clone(),
            limit,
        };
        let result = crate::inference_process::run(
            config,
            request(
                config,
                identity,
                operation,
                lease.clone(),
                &temp.path().join("result.json"),
            ),
            remaining()?,
            &mut |_| {
                remaining()?;
                check()
            },
        )?;
        match result {
            WorkerResult::Candidates { items } => output.vectors = items,
            _ => return Err(AppError::InvalidInput("inference_result_kind")),
        }
    }
    // Only authorized quote text reaches this separate scoring operation. The
    // existing whole-source generation is never reused for a clipped cosine.
    for batch in annotations.chunks(128) {
        check()?;
        let mut spans = Vec::new();
        for evidence in batch {
            for (start, end, text) in pkms_inference::remote_chunks::spans(
                &evidence.text,
                pkms_inference::remote_chunks::BYTE_BUDGET,
            )
            .map_err(AppError::InvalidInput)?
            {
                spans.push((evidence.id.clone(), start, end, text));
            }
        }
        if spans.len() > 16_384 {
            return Err(AppError::InvalidInput("embedding_chunk_limit"));
        }
        let texts = spans
            .iter()
            .map(|(_, _, _, text)| text.clone())
            .collect::<Vec<_>>();
        output.embedding_calls += (texts.len() as u64)
            .div_ceil(pkms_inference::remote_embeddings::MAX_BATCH as u64)
            .max(1);
        output.estimated_embedding_tokens +=
            texts.iter().map(|text| estimate_tokens(text)).sum::<u64>();
        let vectors = crate::inference_embeddings::embed(
            identity,
            &profile,
            crate::inference_embeddings::Kind::Query,
            &texts,
            config,
            remaining()?,
            check,
        )?;
        let mut items = spans
            .into_iter()
            .zip(vectors)
            .map(|((id, start, end, _), vector)| ExactQuoteScore {
                id,
                start_codepoint: start,
                end_codepoint: end,
                cosine_similarity: vector
                    .iter()
                    .zip(&query_vector)
                    .map(|(a, b)| f64::from(*a) * f64::from(*b))
                    .sum::<f64>()
                    .clamp(-1., 1.),
            })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| {
            b.cosine_similarity
                .total_cmp(&a.cosine_similarity)
                .then(a.id.cmp(&b.id))
                .then(a.start_codepoint.cmp(&b.start_codepoint))
        });
        items.truncate(limit);
        output
            .annotations
            .extend(quote_candidates(batch, items, &profile.id, limit)?);
    }
    output.annotations.sort_by(|a, b| {
        b.cosine_similarity
            .total_cmp(&a.cosine_similarity)
            .then(a.evidence.id.cmp(&b.evidence.id))
    });
    output.annotations.truncate(limit);
    remaining()?;
    Ok(output)
}
/// Offline E5 path. The resident worker embeds the query and searches the
/// projection; exact quote scores use the same pinned model, so no network or
/// remote credential is involved.
fn local_candidates(
    library: &Library,
    identity: &WorkspaceIdentity,
    config: &InferenceConfig,
    query: &SearchQuery,
    lease: RequestIdentity,
    timeout: Duration,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<QueryCandidates> {
    let started = Instant::now();
    let remaining = || {
        timeout
            .checked_sub(started.elapsed())
            .filter(|duration| !duration.is_zero())
            .ok_or(AppError::InvalidInput("inference_timeout"))
    };
    let mut output = QueryCandidates::default();
    let Some(profile) = library.active_embedding_profile()? else {
        return Ok(output);
    };
    let annotations = library.annotation_search_inputs(&query.scope)?;
    let active = library.active_index_generations_in_scope(&query.scope)?;
    let limit = (query.limit as usize * 8).clamp(32, 800);
    let identity_for_call = || RequestIdentity {
        request_id: uuid::Uuid::new_v4().to_string(),
        job_id: lease.job_id.clone(),
        attempt: lease.attempt,
        lease_token: lease.lease_token.clone(),
    };
    if !active.is_empty() {
        check()?;
        let generations = active
            .into_iter()
            .filter(|generation| generation.profile_id == profile.id)
            .map(|generation| GenerationExpectation {
                generation_id: generation.id,
                chunk_count: generation.chunk_count,
            })
            .collect::<Vec<_>>();
        if !generations.is_empty() {
            let temp = tempfile::Builder::new()
                .prefix("pkms-local-query-")
                .tempdir()?;
            let request = request(
                config,
                identity,
                Operation::SearchVectors {
                    profile: profile.clone(),
                    generations,
                    query: query.query.clone(),
                    limit,
                },
                identity_for_call(),
                &temp.path().join("search.json"),
            );
            match crate::inference_process::run(config, request, remaining()?, &mut |_| {
                remaining()?;
                check()
            })? {
                WorkerResult::Candidates { items } => output.vectors = items,
                _ => return Err(AppError::InvalidInput("inference_result_kind")),
            }
            output.embedding_calls += 1;
            output.estimated_embedding_tokens += estimate_tokens(&query.query);
        }
    }
    for batch in annotations.chunks(128) {
        check()?;
        let inputs = batch
            .iter()
            .map(|evidence| ExactQuoteInput {
                id: evidence.id.clone(),
                text: evidence.text.clone(),
            })
            .collect::<Vec<_>>();
        let temp = tempfile::Builder::new()
            .prefix("pkms-local-quotes-")
            .tempdir()?;
        let request = request(
            config,
            identity,
            Operation::ScoreExactQuotes {
                profile: profile.clone(),
                query: query.query.clone(),
                inputs,
                limit,
            },
            identity_for_call(),
            &temp.path().join("quotes.json"),
        );
        let items = match crate::inference_process::run(config, request, remaining()?, &mut |_| {
            remaining()?;
            check()
        })? {
            WorkerResult::QuoteScores { items } => items,
            _ => return Err(AppError::InvalidInput("inference_result_kind")),
        };
        output.embedding_calls += 1;
        output.estimated_embedding_tokens += batch
            .iter()
            .map(|evidence| estimate_tokens(&evidence.text))
            .sum::<u64>();
        output
            .annotations
            .extend(quote_candidates(batch, items, &profile.id, limit)?);
    }
    output.annotations.sort_by(|a, b| {
        b.cosine_similarity
            .total_cmp(&a.cosine_similarity)
            .then(a.evidence.id.cmp(&b.evidence.id))
    });
    output.annotations.truncate(limit);
    remaining()?;
    Ok(output)
}

fn quote_candidates(
    batch: &[pkms_core::SearchEvidence],
    items: Vec<ExactQuoteScore>,
    profile_id: &str,
    limit: usize,
) -> Result<Vec<AnnotationVectorCandidate>> {
    let mut output = Vec::new();
    if items.len() > limit {
        return Err(AppError::InvalidInput("inference_result_limit"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        let Some(original) = batch.iter().find(|evidence| evidence.id == item.id) else {
            return Err(AppError::InvalidInput("inference_invalid_quote"));
        };
        if !item.cosine_similarity.is_finite()
            || !(-1.0..=1.0).contains(&item.cosine_similarity)
            || item.start_codepoint >= item.end_codepoint
            || item.end_codepoint as usize > original.text.chars().count()
            || !seen.insert((item.id.clone(), item.start_codepoint, item.end_codepoint))
        {
            return Err(AppError::InvalidInput("inference_invalid_quote"));
        }
        let mut evidence = original.clone();
        evidence.text = original
            .text
            .chars()
            .skip(item.start_codepoint as usize)
            .take((item.end_codepoint - item.start_codepoint) as usize)
            .collect();
        evidence.target.start_codepoint = original
            .target
            .start_codepoint
            .checked_add(item.start_codepoint)
            .ok_or(AppError::InvalidInput("inference_invalid_quote"))?;
        evidence.target.end_codepoint = original
            .target
            .start_codepoint
            .checked_add(item.end_codepoint)
            .ok_or(AppError::InvalidInput("inference_invalid_quote"))?;
        evidence.id = format!(
            "{}:{}:{}",
            original.id, item.start_codepoint, item.end_codepoint
        );
        output.push(AnnotationVectorCandidate {
            evidence,
            profile_id: profile_id.to_owned(),
            cosine_similarity: item.cosine_similarity,
        });
    }
    Ok(output)
}
/// Protocol family used for the embedding credential binding.
pub(crate) const EMBEDDING_PROTOCOL: &str = "openai_embeddings";

/// Clamped so an interrupted or paused call can never write an out-of-range row.
pub(crate) fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .min(3_600_000)
}

impl InferenceConnection {
    /// Measures one interactive retrieval for the statistics panel. Metrics are
    /// best effort: a telemetry write never changes what the user sees.
    pub fn search(
        &mut self,
        config: &InferenceConfig,
        query: SearchQuery,
    ) -> Result<SearchResults> {
        query.validate()?;
        let mode = query.mode;
        let started = Instant::now();
        let result = self.run_search(config, query);
        let count = result
            .as_ref()
            .map_or(0, |results| results.items.len() as u64);
        let _ = self
            .library
            .record_retrieval(mode, elapsed_ms(started), count);
        result
    }

    fn run_search(
        &mut self,
        config: &InferenceConfig,
        query: SearchQuery,
    ) -> Result<SearchResults> {
        // A disabled semantic capability never reaches a remote service; the
        // caller still gets a lexical result plus an explicit reason.
        if query.mode != SearchMode::Lexical
            && !self.library.retrieval_settings()?.params.semantic_enabled
        {
            let mut results = self.library.keyword_search(&query)?;
            if query.mode == SearchMode::Semantic {
                results.items.clear();
            }
            results.semantic_available = false;
            results.unavailable_reason = Some("semantic_disabled".into());
            return Ok(results);
        }
        if query.mode != SearchMode::Lexical {
            let profile = self.library.active_embedding_profile()?;
            let remote = profile
                .as_ref()
                .is_some_and(|profile| profile.spec.provider.is_some());
            if remote && !self.library.retrieval_settings()?.params.remote_consent {
                let mut results = self.library.keyword_search(&query)?;
                if query.mode == SearchMode::Semantic {
                    results.items.clear();
                }
                results.semantic_available = false;
                results.unavailable_reason = Some("remote_consent_required".into());
                return Ok(results);
            }
        } // Final fusion reads current lexical evidence once, after inference, so
        // edits cannot reuse stale results.
        if query.mode == SearchMode::Lexical {
            return Ok(self.library.keyword_search(&query)?);
        }
        let lease = RequestIdentity {
            request_id: uuid::Uuid::new_v4().to_string(),
            job_id: uuid::Uuid::new_v4().to_string(),
            attempt: 1,
            lease_token: uuid::Uuid::new_v4().to_string(),
        };
        match candidates(
            &self.library,
            &self.identity,
            config,
            &query,
            lease,
            &mut || Ok(()),
        ) {
            Ok(items) => {
                let calls = items.embedding_calls;
                let tokens = items.estimated_embedding_tokens;
                let results = self.library.search_with_annotation_vectors(
                    &query,
                    items.vectors,
                    items.annotations,
                )?;
                self.record_embedding_usage(calls, tokens);
                Ok(results)
            }
            Err(error) => {
                // Inference may have taken seconds: revalidate the lexical fallback
                // against the current lifecycle, content revision and scope.
                let mut results = self.library.keyword_search(&query)?;
                if query.mode == SearchMode::Semantic {
                    results.items.clear();
                }
                results.semantic_available = false;
                results.unavailable_reason = Some(error_code(&error).into());
                Ok(results)
            }
        }
    }

    fn record_embedding_usage(&mut self, calls: u64, tokens: u64) {
        record_embedding_usage(&mut self.library, calls, tokens);
    }
}

/// Records one embedding workload: the call count is derived from the provider
/// batch size and the token count is the documented local estimate.
pub(crate) fn record_embedding_texts(library: &mut Library, texts: &[String]) {
    if texts.is_empty() {
        return;
    }
    let calls = (texts.len() as u64)
        .div_ceil(pkms_inference::remote_embeddings::MAX_BATCH as u64)
        .max(1);
    let tokens = texts.iter().map(|text| estimate_tokens(text)).sum::<u64>();
    record_embedding_usage(library, calls, tokens);
}

/// Embedding providers report no usage on this transport, so the count is
/// recorded as an estimate and attributed to the active profile. Best effort:
/// telemetry never fails the caller's retrieval.
pub(crate) fn record_embedding_usage(library: &mut Library, calls: u64, tokens: u64) {
    if calls == 0 {
        return;
    }
    let Ok(Some(profile)) = library.active_embedding_profile() else {
        return;
    };
    let _ = library.record_model_usage(
        pkms_core::UsagePurpose::Embedding,
        EMBEDDING_PROTOCOL,
        &profile.spec.model,
        pkms_core::ModelUsage {
            input_tokens: Some(tokens),
            output_tokens: None,
            estimated: true,
            latency: None,
        },
    );
}
/// Records the save-time probe: one call, with the provider-reported token count
/// when the provider returns one and "unknown" when it does not.
pub(crate) fn record_embedding_probe(library: &mut Library, model: &str, tokens: Option<u64>) {
    let _ = library.record_model_usage(
        pkms_core::UsagePurpose::Embedding,
        EMBEDDING_PROTOCOL,
        model,
        pkms_core::ModelUsage {
            input_tokens: tokens,
            output_tokens: None,
            estimated: tokens.is_none(),
            latency: None,
        },
    );
}
pub(crate) fn error_code(error: &AppError) -> &'static str {
    crate::error_codes::error_code(error, crate::error_codes::ErrorDomain::Inference)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pkms_core::{AnnotationEvidence, EvidenceKind, EvidenceTarget, SearchEvidence};

    #[test]
    fn malformed_search_is_rejected_before_remote_preparation() {
        let root = tempfile::tempdir().unwrap();
        let library = Library::open(root.path().join("knowledge")).unwrap();
        let identity = WorkspaceIdentity {
            local_instance_id: uuid::Uuid::new_v4().to_string(),
            workspace_id: library.info().unwrap().workspace_id,
        };
        let config = InferenceConfig {
            executable: root.path().join("absent-worker.exe"),
            model_directory: root.path().join("models"),
            vector_root: root.path().join("vectors"),
            shutdown: Default::default(),
        };
        let mut query = SearchQuery {
            query: "合法问题".into(),
            mode: SearchMode::Hybrid,
            scope: Default::default(),
            limit: 8,
            minimum_cosine: None,
        };
        let mut cases = Vec::new();
        query.limit = 101;
        cases.push(query.clone());
        query.limit = 8;
        query.scope.collection_id = Some("invalid".into());
        cases.push(query.clone());
        query.scope.collection_id = None;
        query.minimum_cosine = Some(f64::NAN);
        cases.push(query);
        for invalid in cases {
            let lease = RequestIdentity {
                request_id: uuid::Uuid::new_v4().to_string(),
                job_id: uuid::Uuid::new_v4().to_string(),
                attempt: 1,
                lease_token: uuid::Uuid::new_v4().to_string(),
            };
            let mut prepared = false;
            let result = candidates_with_timeout(
                &library,
                &identity,
                &config,
                &invalid,
                lease,
                Duration::from_secs(1),
                &mut || {
                    prepared = true;
                    Ok(())
                },
            );
            assert!(matches!(
                result,
                Err(AppError::Core(pkms_core::Error::InvalidInput(_)))
            ));
            assert!(
                !prepared,
                "invalid request must stop before remote preparation"
            );
        }
    }

    fn evidence() -> SearchEvidence {
        SearchEvidence {
            id: "annotation:grant".into(),
            target: EvidenceTarget {
                object_id: "source".into(),
                kind: EvidenceKind::Source,
                content_revision: "version".into(),
                metadata_revision: 1,
                parse_id: None,
                block_id: "block".into(),
                start_codepoint: 100,
                end_codepoint: 104,
                page_index: None,
            },
            title: "摘录".into(),
            text: "甲😀e\u{301}".into(),
            lexical_rank: None,
            cosine_similarity: None,
            fusion_rank: None,
            generation_id: None,
            annotation: Some(AnnotationEvidence {
                annotation_id: "grant".into(),
                annotation_revision: 1,
                locator_hash: "hash".into(),
                embedding_profile_id: None,
            }),
        }
    }
    fn score() -> ExactQuoteScore {
        ExactQuoteScore {
            id: "annotation:grant".into(),
            start_codepoint: 1,
            end_codepoint: 4,
            cosine_similarity: 0.8,
        }
    }
    #[test]
    fn quote_worker_scores_select_only_authorized_unicode_subranges() {
        let candidates = quote_candidates(&[evidence()], vec![score()], "profile", 1).unwrap();
        assert_eq!(candidates[0].evidence.text, "😀e\u{301}");
        assert_eq!(candidates[0].evidence.target.start_codepoint, 101);
        assert_eq!(candidates[0].evidence.target.end_codepoint, 104);
        assert_eq!(candidates[0].profile_id, "profile");
    }
    #[test]
    fn quote_worker_invalid_ids_ranges_scores_and_duplicates_are_rejected() {
        let invalid = [
            ExactQuoteScore {
                id: "outside".into(),
                ..score()
            },
            ExactQuoteScore {
                start_codepoint: 4,
                ..score()
            },
            ExactQuoteScore {
                end_codepoint: 5,
                ..score()
            },
            ExactQuoteScore {
                cosine_similarity: f64::NAN,
                ..score()
            },
            ExactQuoteScore {
                cosine_similarity: 1.01,
                ..score()
            },
        ];
        for item in invalid {
            assert!(quote_candidates(&[evidence()], vec![item], "profile", 1).is_err());
        }
        assert!(quote_candidates(&[evidence()], vec![score(), score()], "profile", 2).is_err());
        assert!(quote_candidates(&[evidence()], vec![score()], "profile", 0).is_err());
        let mut overflowing = evidence();
        overflowing.target.start_codepoint = u32::MAX;
        assert!(quote_candidates(&[overflowing], vec![score()], "profile", 1).is_err());
    }
}

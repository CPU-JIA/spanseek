//! Pinned local E5 inference. Loading never downloads files or truncates text.
use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use pkms_core::{EmbeddingProfile, IndexChunkInput, IndexInput, IndexPlan};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};
use tokenizers::Tokenizer;

pub use crate::profile::{
    DIMENSIONS, MAX_TOKENS, MODEL_HASH, MODEL_REVISION, TOKENIZER_HASH, profile_spec,
};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EmbeddingError(pub &'static str);
type Result<T> = std::result::Result<T, EmbeddingError>;

pub const MAX_EXACT_QUOTE_INPUTS: usize = 128;
pub const MAX_EXACT_QUOTE_BYTES: usize = 64 * 1024;
pub const MAX_EXACT_QUOTE_TOTAL_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EXACT_QUOTE_CHUNKS: usize = 16_384;
pub const MAX_EXACT_QUOTE_RESULTS: usize = 800;

pub fn validate_exact_quote_request(
    query: &str,
    inputs: &[crate::protocol::ExactQuoteInput],
    limit: usize,
) -> Result<()> {
    if query.trim().is_empty()
        || query.len() > 4096
        || query.contains('\0')
        || !(1..=MAX_EXACT_QUOTE_RESULTS).contains(&limit)
    {
        return Err(EmbeddingError("embedding_invalid_input"));
    }
    validate_exact_quote_inputs(inputs)
}

fn validate_exact_quote_inputs(inputs: &[crate::protocol::ExactQuoteInput]) -> Result<()> {
    if inputs.is_empty() || inputs.len() > MAX_EXACT_QUOTE_INPUTS {
        return Err(EmbeddingError("embedding_invalid_input"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut total = 0usize;
    for input in inputs {
        if input.id.is_empty()
            || input.id.len() > 128
            || input.id.chars().any(char::is_control)
            || !ids.insert(&input.id)
            || input.text.trim().is_empty()
            || input.text.contains('\0')
        {
            return Err(EmbeddingError("embedding_invalid_input"));
        }
        total = total
            .checked_add(input.text.len())
            .ok_or(EmbeddingError("embedding_input_limit"))?;
        if input.text.len() > MAX_EXACT_QUOTE_BYTES || total > MAX_EXACT_QUOTE_TOTAL_BYTES {
            return Err(EmbeddingError("embedding_input_limit"));
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct ExactQuoteChunk {
    pub id: String,
    pub text: String,
    pub start_codepoint: u32,
    pub end_codepoint: u32,
}

fn validate_profile(profile: &EmbeddingProfile) -> Result<()> {
    if serde_json::to_value(&profile.spec).ok() != serde_json::to_value(profile_spec()).ok() {
        return Err(EmbeddingError("embedding_profile_mismatch"));
    }
    Ok(())
}
fn verified(path: &Path, name: &str, hash: &str, max_bytes: u64) -> Result<Vec<u8>> {
    let path = path.join(name);
    let metadata =
        fs::metadata(&path).map_err(|_| EmbeddingError("embedding_model_unavailable"))?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(EmbeddingError("embedding_model_invalid"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| EmbeddingError("embedding_model_unavailable"))?
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| EmbeddingError("embedding_model_unavailable"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(EmbeddingError("embedding_model_invalid"));
    }
    if format!("{:x}", Sha256::digest(&bytes)) != hash {
        return Err(EmbeddingError("embedding_model_hash_mismatch"));
    }
    Ok(bytes)
}
pub struct Chunker {
    tokenizer: Tokenizer,
}
impl Chunker {
    /// Splits only the supplied exact quote. No source title, context or newline
    /// is added to the E5 passage prefix; offsets refer to the original input.
    pub fn prepare_exact_quotes(
        &self,
        profile: &EmbeddingProfile,
        inputs: &[crate::protocol::ExactQuoteInput],
    ) -> Result<Vec<ExactQuoteChunk>> {
        validate_profile(profile)?;
        validate_exact_quote_inputs(inputs)?;
        let prefix = &profile.spec.passage_prefix;
        let overhead = self.token_count(prefix)?;
        if overhead + 8 >= MAX_TOKENS {
            return Err(EmbeddingError("embedding_token_budget"));
        }
        let mut chunks = Vec::new();
        for input in inputs {
            let encoding = self
                .tokenizer
                .encode(input.text.as_str(), false)
                .map_err(|_| EmbeddingError("embedding_tokenization_failed"))?;
            let mut boundaries: Vec<usize> = encoding
                .get_offsets()
                .iter()
                .map(|(_, end)| *end)
                .filter(|end| {
                    *end > 0 && *end <= input.text.len() && input.text.is_char_boundary(*end)
                })
                .collect();
            boundaries.push(input.text.len());
            boundaries.sort_unstable();
            boundaries.dedup();
            let mut start = 0;
            let mut start_cp = 0;
            while start < input.text.len() {
                let first = boundaries.partition_point(|end| *end <= start);
                let mut index = (first + MAX_TOKENS - overhead - 8).min(boundaries.len() - 1);
                let end = loop {
                    let end = boundaries[index];
                    if self.token_count(&format!("{prefix}{}", &input.text[start..end]))?
                        <= MAX_TOKENS
                    {
                        break end;
                    }
                    if index == first {
                        return Err(EmbeddingError("embedding_token_budget"));
                    }
                    index -= 1;
                };
                let text = input.text[start..end].to_owned();
                let end_cp = start_cp + text.chars().count() as u32;
                chunks.push(ExactQuoteChunk {
                    id: input.id.clone(),
                    text,
                    start_codepoint: start_cp,
                    end_codepoint: end_cp,
                });
                if chunks.len() > MAX_EXACT_QUOTE_CHUNKS {
                    return Err(EmbeddingError("embedding_chunk_limit"));
                }
                start = end;
                start_cp = end_cp;
            }
        }
        Ok(chunks)
    }
    pub fn load(directory: &Path) -> Result<Self> {
        let bytes = verified(
            directory,
            "tokenizer.json",
            TOKENIZER_HASH,
            20 * 1024 * 1024,
        )?;
        let mut tokenizer = Tokenizer::from_bytes(bytes)
            .map_err(|_| EmbeddingError("embedding_tokenizer_invalid"))?;
        tokenizer
            .with_truncation(None)
            .map_err(|_| EmbeddingError("embedding_tokenizer_invalid"))?;
        tokenizer.with_padding(None);
        Ok(Self { tokenizer })
    }
    pub fn token_count(&self, text: &str) -> Result<usize> {
        self.tokenizer
            .encode(text, true)
            .map(|e| e.len())
            .map_err(|_| EmbeddingError("embedding_tokenization_failed"))
    }
    pub fn prepare(&self, profile: &EmbeddingProfile, input: &IndexInput) -> Result<IndexPlan> {
        validate_profile(profile)?;
        let prefix = format!("{}{}\n", profile.spec.passage_prefix, input.title);
        let overhead = self.token_count(&prefix)?;
        if overhead + 8 >= MAX_TOKENS {
            return Err(EmbeddingError("embedding_title_token_limit"));
        }
        let mut chunks = Vec::new();
        for block in &input.blocks {
            if block.text.len() > 2 * 1024 * 1024 {
                return Err(EmbeddingError("embedding_input_limit"));
            }
            if block.text.is_empty() {
                continue;
            }
            let encoding = self
                .tokenizer
                .encode(block.text.as_str(), false)
                .map_err(|_| EmbeddingError("embedding_tokenization_failed"))?;
            let mut boundaries: Vec<usize> = encoding
                .get_offsets()
                .iter()
                .map(|(_, end)| *end)
                .filter(|end| {
                    *end > 0 && *end <= block.text.len() && block.text.is_char_boundary(*end)
                })
                .collect();
            boundaries.push(block.text.len());
            boundaries.sort_unstable();
            boundaries.dedup();
            let mut start = 0;
            let mut start_cp = 0;
            while start < block.text.len() {
                let first = boundaries.partition_point(|end| *end <= start);
                let mut index = (first + MAX_TOKENS - overhead - 8).min(boundaries.len() - 1);
                let (end, token_count) = loop {
                    let end = boundaries[index];
                    let count =
                        self.token_count(&format!("{prefix}{}", &block.text[start..end]))?;
                    if count <= MAX_TOKENS {
                        break (end, count);
                    }
                    if index == first {
                        return Err(EmbeddingError("embedding_token_budget"));
                    }
                    index -= 1;
                };
                let text = block.text[start..end].to_owned();
                if text.len() > 64 * 1024 {
                    return Err(EmbeddingError("embedding_chunk_size_limit"));
                }
                let end_cp = start_cp + text.chars().count() as u32;
                let key = serde_json::to_vec(&(
                    &profile.id,
                    &input.object_id,
                    &input.content_revision,
                    &block.target.block_id,
                    start_cp,
                    end_cp,
                ))
                .map_err(|_| EmbeddingError("embedding_invalid_input"))?;
                chunks.push(IndexChunkInput {
                    chunk_id: format!("{:x}", Sha256::digest(key)),
                    block_id: block.target.block_id.clone(),
                    start_codepoint: start_cp,
                    end_codepoint: end_cp,
                    text,
                    token_count: token_count as u32,
                });
                if chunks.len() > 100_000 {
                    return Err(EmbeddingError("embedding_chunk_limit"));
                }
                start = end;
                start_cp = end_cp;
            }
        }
        Ok(IndexPlan {
            profile_id: profile.id.clone(),
            object_id: input.object_id.clone(),
            content_revision: input.content_revision.clone(),
            metadata_revision: input.metadata_revision,
            parser_revision: input.parser_revision.clone(),
            chunks,
        })
    }
}

pub struct Embedder {
    model: TextEmbedding,
    chunker: Chunker,
}
impl Embedder {
    pub fn score_exact_quotes(
        &mut self,
        profile: &EmbeddingProfile,
        query: &str,
        inputs: &[crate::protocol::ExactQuoteInput],
        limit: usize,
        progress: &mut dyn FnMut(u32, u32) -> Result<()>,
    ) -> Result<Vec<crate::protocol::ExactQuoteScore>> {
        validate_exact_quote_request(query, inputs, limit)?;
        let chunks = self.chunker.prepare_exact_quotes(profile, inputs)?;
        progress(0, chunks.len() as u32)?;
        let query_vector = self
            .embed(vec![format!("{}{query}", profile.spec.query_prefix)])?
            .pop()
            .ok_or(EmbeddingError("embedding_invalid_vector_count"))?;
        let query_norm = query_vector
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            .sqrt();
        let mut scores = Vec::with_capacity(chunks.len());
        for batch in chunks.chunks(8) {
            let vectors = self.embed(
                batch
                    .iter()
                    .map(|chunk| format!("{}{}", profile.spec.passage_prefix, chunk.text))
                    .collect(),
            )?;
            for (chunk, vector) in batch.iter().zip(vectors) {
                let dot = vector
                    .iter()
                    .zip(&query_vector)
                    .map(|(a, b)| f64::from(*a) * f64::from(*b))
                    .sum::<f64>();
                let norm = vector
                    .iter()
                    .map(|value| f64::from(*value).powi(2))
                    .sum::<f64>()
                    .sqrt();
                let cosine = dot / (norm * query_norm);
                if !cosine.is_finite() {
                    return Err(EmbeddingError("embedding_invalid_vector"));
                }
                scores.push(crate::protocol::ExactQuoteScore {
                    id: chunk.id.clone(),
                    start_codepoint: chunk.start_codepoint,
                    end_codepoint: chunk.end_codepoint,
                    cosine_similarity: cosine.clamp(-1.0, 1.0),
                });
            }
            progress(scores.len() as u32, chunks.len() as u32)?;
        }
        scores.sort_by(|a, b| {
            b.cosine_similarity
                .total_cmp(&a.cosine_similarity)
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.start_codepoint.cmp(&b.start_codepoint))
                .then_with(|| a.end_codepoint.cmp(&b.end_codepoint))
        });
        scores.truncate(limit);
        Ok(scores)
    }
    pub fn load(directory: &Path, profile: &EmbeddingProfile) -> Result<Self> {
        validate_profile(profile)?;
        let files = TokenizerFiles {
            tokenizer_file: verified(
                directory,
                "tokenizer.json",
                TOKENIZER_HASH,
                20 * 1024 * 1024,
            )?,
            config_file: verified(
                directory,
                "config.json",
                "bbb7c1333fc4b3e27fbc9cd5d2070aabcc1d4dfb99917c3633e772f97545a6b6",
                1024 * 1024,
            )?,
            special_tokens_map_file: verified(
                directory,
                "special_tokens_map.json",
                "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
                1024 * 1024,
            )?,
            tokenizer_config_file: verified(
                directory,
                "tokenizer_config.json",
                "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
                1024 * 1024,
            )?,
        };
        let onnx = verified(directory, "model.onnx", MODEL_HASH, 500 * 1024 * 1024)?;
        let mut model = TextEmbedding::try_new_from_user_defined(
            UserDefinedEmbeddingModel::new(onnx, files).with_pooling(Pooling::Mean),
            InitOptionsUserDefined::new()
                .with_max_length(MAX_TOKENS)
                .with_intra_threads(4),
        )
        .map_err(|_| EmbeddingError("embedding_model_load_failed"))?;
        // The upstream loader enables truncation: explicitly disable it, then
        // validate every exact model input before inference.
        model
            .tokenizer
            .with_truncation(None)
            .map_err(|_| EmbeddingError("embedding_tokenizer_invalid"))?;
        let mut counter = model.tokenizer.clone();
        counter.with_padding(None);
        let chunker = Chunker { tokenizer: counter };
        Ok(Self { model, chunker })
    }
    pub fn embed(&mut self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() || texts.len() > 32 {
            return Err(EmbeddingError("embedding_batch_limit"));
        }
        for text in &texts {
            if self.chunker.token_count(text)? > MAX_TOKENS {
                return Err(EmbeddingError("embedding_token_budget"));
            }
        }
        let expected = texts.len();
        let mut vectors = self
            .model
            .embed(texts, Some(8))
            .map_err(|_| EmbeddingError("embedding_inference_failed"))?;
        if vectors.len() != expected {
            return Err(EmbeddingError("embedding_invalid_vector_count"));
        }
        for vector in &mut vectors {
            if vector.len() != DIMENSIONS || vector.iter().any(|v| !v.is_finite()) {
                return Err(EmbeddingError("embedding_invalid_vector"));
            }
            let norm = vector
                .iter()
                .map(|v| f64::from(*v).powi(2))
                .sum::<f64>()
                .sqrt();
            if !norm.is_finite() || norm < 1e-12 {
                return Err(EmbeddingError("embedding_invalid_vector"));
            }
            for value in vector {
                *value = (f64::from(*value) / norm) as f32;
            }
        }
        Ok(vectors)
    }
}

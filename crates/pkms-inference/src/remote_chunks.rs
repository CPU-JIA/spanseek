//! Deterministic remote chunks. Boundaries follow the structure the parsers
//! already produced: a chunk never crosses a block (paragraph, heading, list
//! item, table cell or PDF page block), and a block that does not fit is split at
//! line and sentence boundaries instead of mid-word. Consecutive parts of one
//! block carry the configured overlap so a hit near a boundary still sees its
//! context, and every chunk stays inside the configured token cap.
use pkms_core::{
    EmbeddingProfile, EmbeddingProfileSpec, EmbeddingProvider, IndexChunkInput, IndexInput,
    IndexPlan,
};
use sha2::{Digest, Sha256};

pub const MODEL: &str = "@cf/baai/bge-m3";
pub const REVISION: &str = "cloudflare-hosted-bge-m3-v1";
pub const DIMENSIONS: u32 = 1024;
/// Highest vector size this build accepts from any embedding model.
pub const MAX_DIMENSIONS: u32 = 4096;
/// Byte budget for the exact-quote path, which is unrelated to indexing chunks.
pub const BYTE_BUDGET: usize = 2048;
pub const DEFAULT_CHUNK_TOKENS: u32 = 512;
pub const MIN_CHUNK_TOKENS: u32 = 64;
pub const MAX_CHUNK_TOKENS: u32 = 1024;
pub const DEFAULT_OVERLAP_PERCENT: u32 = 15;
pub const MAX_OVERLAP_PERCENT: u32 = 30;
pub const TOKENIZER: &str = "pkms-estimated-tokens-v1";
pub const CHUNKER: &str = "pkms-structure-first-v2:block-line-sentence:overlap";
const MAX_TITLE_BYTES: usize = 1024;

/// Deterministic, machine-independent token estimate: every non-ASCII character
/// (CJK, emoji, punctuation) counts as one token and every four ASCII bytes count
/// as one. It deliberately over-counts rather than under-counts, and it never
/// depends on a tokenizer file.
pub fn estimate_tokens(text: &str) -> u32 {
    let mut tokens = 0u32;
    let mut ascii = 0u32;
    for character in text.chars() {
        if character.is_ascii() {
            ascii += 1;
        } else {
            tokens += ascii.div_ceil(4) + 1;
            ascii = 0;
        }
    }
    tokens + ascii.div_ceil(4)
}

/// Overlap for a chunk cap, expressed in tokens and capped at half the chunk so a
/// chunk can never be pure overlap.
pub fn overlap_for(chunk_tokens: u32, overlap_percent: u32) -> u32 {
    let percent = overlap_percent.min(MAX_OVERLAP_PERCENT);
    (chunk_tokens * percent / 100).min(chunk_tokens / 2)
}

/// A remote embedding configuration: which provider and model is called, with
/// which vector size, plus the chunk cap that shares the same profile identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteProfile {
    pub provider: EmbeddingProvider,
    pub endpoint: String,
    pub model: String,
    /// Vector size the provider actually returned, measured when it was saved.
    pub dimensions: u32,
    /// Output size the user asked for, when the model supports truncation.
    pub output_dimensions: Option<u32>,
    pub chunk_tokens: u32,
    pub overlap_tokens: u32,
}

pub fn provider_label(provider: EmbeddingProvider) -> &'static str {
    match provider {
        EmbeddingProvider::Cloudflare => "cloudflare",
        EmbeddingProvider::Openai => "openai",
        EmbeddingProvider::Gemini => "gemini",
        EmbeddingProvider::OpenaiCompatible => "openai-compatible",
    }
}

impl RemoteProfile {
    /// The shipped default: Cloudflare Workers AI with BGE-M3.
    pub fn cloudflare(endpoint: String, chunk_tokens: u32, overlap_tokens: u32) -> Self {
        Self {
            provider: EmbeddingProvider::Cloudflare,
            endpoint,
            model: MODEL.into(),
            dimensions: DIMENSIONS,
            output_dimensions: None,
            chunk_tokens,
            overlap_tokens,
        }
    }
    /// Profiles registered before V1.7 stored no provider and no output size.
    /// Their JSON must stay byte-identical, so their identity — and the index
    /// built with it — survives an upgrade without a rebuild.
    fn is_default_cloudflare(&self) -> bool {
        self.provider == EmbeddingProvider::Cloudflare
            && self.model == MODEL
            && self.dimensions == DIMENSIONS
            && self.output_dimensions.is_none()
    }
    fn revision(&self) -> String {
        if self.is_default_cloudflare() {
            REVISION.into()
        } else {
            format!("{}:{}:v1", provider_label(self.provider), self.model)
        }
    }
    pub fn spec(&self) -> EmbeddingProfileSpec {
        let default = self.is_default_cloudflare();
        EmbeddingProfileSpec {
            endpoint: Some(self.endpoint.clone()),
            model: self.model.clone(),
            revision: self.revision(),
            dimensions: self.dimensions,
            normalized: true,
            query_prefix: String::new(),
            passage_prefix: String::new(),
            tokenizer: TOKENIZER.into(),
            chunker: CHUNKER.into(),
            max_tokens: self.chunk_tokens,
            overlap_tokens: self.overlap_tokens,
            provider: (!default).then_some(self.provider),
            output_dimensions: self.output_dimensions,
        }
    }
}

pub fn profile_spec(remote: &RemoteProfile) -> EmbeddingProfileSpec {
    remote.spec()
}

/// The profile a library gets before it configures its own chunk cap.
pub fn default_profile_spec(endpoint: String) -> EmbeddingProfileSpec {
    RemoteProfile::cloudflare(
        endpoint,
        DEFAULT_CHUNK_TOKENS,
        overlap_for(DEFAULT_CHUNK_TOKENS, DEFAULT_OVERLAP_PERCENT),
    )
    .spec()
}

/// Read a stored profile back, or `None` when this build did not produce it
/// (different model, dimensions, prefixes, tokenizer or chunker). The chunk cap
/// and its overlap are the only free values.
pub fn remote(profile: &EmbeddingProfile) -> Option<RemoteProfile> {
    let spec = &profile.spec;
    let endpoint = spec.endpoint.clone()?;
    if !(MIN_CHUNK_TOKENS..=MAX_CHUNK_TOKENS).contains(&spec.max_tokens)
        || spec.overlap_tokens > spec.max_tokens / 2
        || !(1..=MAX_DIMENSIONS).contains(&spec.dimensions)
        || spec
            .output_dimensions
            .is_some_and(|value| value != spec.dimensions)
        || spec.model.trim().is_empty()
        || spec.model.len() > 1024
        || spec.revision.trim().is_empty()
    {
        return None;
    }
    let candidate = RemoteProfile {
        provider: spec.provider.unwrap_or(EmbeddingProvider::Cloudflare),
        endpoint,
        model: spec.model.clone(),
        dimensions: spec.dimensions,
        output_dimensions: spec.output_dimensions,
        chunk_tokens: spec.max_tokens,
        overlap_tokens: spec.overlap_tokens,
    };
    (candidate.spec() == *spec).then_some(candidate)
}

pub fn valid(profile: &EmbeddingProfile) -> bool {
    remote(profile).is_some()
}

fn is_sentence_end(character: char) -> bool {
    matches!(
        character,
        '。' | '！' | '？' | '；' | '…' | '!' | '?' | ';' | '．' | '：' | ':'
    )
}

fn is_closing(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '”' | '’' | '」' | '』' | '）' | ')' | '】' | '〕' | '》' | '〉'
    )
}

/// Structure boundaries inside one block: a line break ends a unit, and so does a
/// sentence ender followed by whitespace or the end of the block. Closing
/// punctuation and the following space stay with the sentence that ended.
fn segment_ranges(text: &str) -> Vec<(usize, usize)> {
    let characters = text.char_indices().collect::<Vec<_>>();
    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    while index < characters.len() {
        let (_, character) = characters[index];
        let lookahead = characters.get(index + 1).map(|(_, next)| *next);
        let boundary = character == '\n'
            || is_sentence_end(character)
            || (character == '.'
                && lookahead.is_none_or(|next| next.is_whitespace() || next == '\n'));
        if !boundary {
            index += 1;
            continue;
        }
        let mut end_index = index + 1;
        while let Some((_, next)) = characters.get(end_index) {
            if is_closing(*next) || *next == ' ' || *next == '\t' {
                end_index += 1;
            } else {
                break;
            }
        }
        let end = characters
            .get(end_index)
            .map(|(offset, _)| *offset)
            .unwrap_or(text.len());
        if end > start {
            ranges.push((start, end));
        }
        start = end;
        index = end_index;
    }
    if start < text.len() {
        ranges.push((start, text.len()));
    }
    ranges
}

/// Walks back from `end` until the slice holds at least `tokens` worth of text,
/// never crossing `floor`. Used for both block-level and hard-split overlap.
fn overlap_start(text: &str, floor: usize, end: usize, tokens: u32) -> usize {
    if tokens == 0 || floor >= end {
        return end;
    }
    let mut start = end;
    let mut collected = 0u32;
    for (offset, character) in text[floor..end].char_indices().rev() {
        if collected >= tokens {
            break;
        }
        collected += if character.is_ascii() { 1 } else { 4 };
        start = floor + offset;
    }
    start
}

/// Byte offset of the longest prefix of `text[from..to]` that fits in `budget`.
/// Always advances by at least one character, so a single character that exceeds
/// the whole budget is still emitted instead of looping forever.
fn cap_prefix(text: &str, from: usize, to: usize, budget: u32) -> usize {
    let mut committed = 0u32;
    let mut ascii = 0u32;
    let mut end = from;
    for (offset, character) in text[from..to].char_indices() {
        let total = if character.is_ascii() {
            committed + (ascii + 1).div_ceil(4)
        } else {
            committed + ascii.div_ceil(4) + 1
        };
        if total > budget && end > from {
            break;
        }
        if character.is_ascii() {
            ascii += 1;
        } else {
            committed += ascii.div_ceil(4) + 1;
            ascii = 0;
        }
        end = from + offset + character.len_utf8();
        if total > budget {
            break;
        }
    }
    end
}

/// Splits one oversized unit at the token cap without dropping anything. Parts are
/// contiguous and may overlap by the configured amount.
fn hard_split(
    text: &str,
    start: usize,
    end: usize,
    budget: u32,
    overlap: u32,
) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let part_end = cap_prefix(text, cursor, end, budget);
        parts.push((cursor, part_end));
        if part_end >= end {
            break;
        }
        let next = overlap_start(text, cursor, part_end, overlap);
        cursor = if next > cursor { next } else { part_end };
    }
    parts
}

/// Splits one block into chunks: structural units are packed while they fit, an
/// oversized unit is hard-split, and every part after the first repeats the tail
/// of the previous part as overlap. Parts are contiguous (no gap), so the plan
/// still covers every character of the block.
pub fn split_block(text: &str, budget: u32, overlap: u32) -> Vec<(u32, u32, String)> {
    if text.is_empty() || budget == 0 {
        return Vec::new();
    }
    let overlap = overlap.min(budget / 2);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    for (start, end) in segment_ranges(text) {
        let unit = &text[start..end];
        if estimate_tokens(unit) > budget {
            if let Some(part) = current.take() {
                ranges.push(part);
            }
            ranges.extend(hard_split(text, start, end, budget, overlap));
            continue;
        }
        match current {
            Some((part_start, part_end)) => {
                let room = if ranges.is_empty() {
                    budget
                } else {
                    budget - overlap
                };
                if estimate_tokens(&text[part_start..end]) <= room {
                    current = Some((part_start, end));
                } else {
                    ranges.push((part_start, part_end));
                    current = Some((start, end));
                }
            }
            None => current = Some((start, end)),
        }
    }
    if let Some(part) = current {
        ranges.push(part);
    }
    let offsets = text
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .collect::<Vec<_>>();
    let mut parts = Vec::with_capacity(ranges.len());
    let mut previous_end = 0usize;
    for (index, (start, end)) in ranges.iter().copied().enumerate() {
        let start = if index == 0 {
            start
        } else {
            overlap_start(text, previous_end, start, overlap).min(start)
        };
        previous_end = end;
        parts.push((
            offsets.partition_point(|offset| *offset < start) as u32,
            offsets.partition_point(|offset| *offset < end) as u32,
            text[start..end].to_owned(),
        ));
    }
    parts
}

pub fn spans(text: &str, budget: usize) -> Result<Vec<(u32, u32, String)>, &'static str> {
    if budget < 4 {
        return Err("embedding_token_budget");
    }
    let mut spans = Vec::new();
    let mut start = 0;
    let mut cp = 0;
    while start < text.len() {
        let mut end = (start + budget).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let part = &text[start..end];
        let next = cp + part.chars().count() as u32;
        spans.push((cp, next, part.to_owned()));
        cp = next;
        start = end;
    }
    Ok(spans)
}

pub fn prepare(profile: &EmbeddingProfile, input: IndexInput) -> Result<IndexPlan, &'static str> {
    if !valid(profile) {
        return Err("embedding_profile_mismatch");
    }
    let prefix = format!("{}\n", input.title);
    let prefix_tokens = estimate_tokens(&prefix);
    if prefix.len() > MAX_TITLE_BYTES || prefix_tokens >= profile.spec.max_tokens {
        return Err("embedding_title_token_limit");
    }
    let budget = profile.spec.max_tokens - prefix_tokens;
    let overlap = profile.spec.overlap_tokens.min(budget / 2);
    let mut chunks = Vec::new();
    for block in input.blocks {
        if block.text.is_empty() {
            continue;
        }
        for (start, end, text) in split_block(&block.text, budget, overlap) {
            let key = serde_json::to_vec(&(
                &profile.id,
                &input.object_id,
                &input.content_revision,
                &block.target.block_id,
                start,
                end,
            ))
            .map_err(|_| "embedding_invalid_input")?;
            chunks.push(IndexChunkInput {
                chunk_id: format!("{:x}", Sha256::digest(key)),
                block_id: block.target.block_id.clone(),
                start_codepoint: start,
                end_codepoint: end,
                token_count: prefix_tokens + estimate_tokens(&text),
                text,
            });
            if chunks.len() > 100_000 {
                return Err("embedding_chunk_limit");
            }
        }
    }
    Ok(IndexPlan {
        profile_id: profile.id.clone(),
        object_id: input.object_id,
        content_revision: input.content_revision,
        metadata_revision: input.metadata_revision,
        parser_revision: input.parser_revision,
        chunks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pkms_core::{EvidenceKind, EvidenceTarget, IndexInputBlock};

    fn input(title: &str, blocks: &[(&str, &str)]) -> IndexInput {
        IndexInput {
            object_id: "object".into(),
            kind: EvidenceKind::Note,
            content_revision: "content".into(),
            metadata_revision: 1,
            parser_revision: "parser".into(),
            title: title.into(),
            blocks: blocks
                .iter()
                .map(|(block_id, text)| IndexInputBlock {
                    target: EvidenceTarget {
                        object_id: "object".into(),
                        kind: EvidenceKind::Note,
                        content_revision: "content".into(),
                        metadata_revision: 1,
                        parse_id: None,
                        block_id: (*block_id).into(),
                        start_codepoint: 0,
                        end_codepoint: 0,
                        page_index: None,
                    },
                    text: (*text).into(),
                })
                .collect(),
        }
    }

    fn profile() -> EmbeddingProfile {
        EmbeddingProfile {
            id: "p".into(),
            spec: default_profile_spec("https://example.invalid/v1".into()),
        }
    }

    /// The plan has to tile the block: no gap, no empty part, and each part's text
    /// must be exactly its own range.
    fn contiguous(text: &str, parts: &[(u32, u32, String)]) {
        let characters = text.chars().collect::<Vec<_>>();
        let mut covered = 0u32;
        for (start, end, part) in parts {
            assert!(*start <= covered, "gap before {start}");
            assert!(start < end, "empty part");
            assert_eq!(
                part.as_str(),
                characters[*start as usize..*end as usize]
                    .iter()
                    .collect::<String>(),
                "part text must match its own range"
            );
            covered = covered.max(*end);
        }
        assert_eq!(
            covered as usize,
            characters.len(),
            "the plan must cover the whole block"
        );
    }

    #[test]
    fn long_blocks_split_on_sentence_boundaries_with_overlap_and_a_cap() {
        let sentence = "知识库检索质量取决于分块边界。";
        let text = sentence.repeat(80);
        let parts = split_block(&text, 128, 19);
        assert!(parts.len() > 1, "a long block must split");
        contiguous(&text, &parts);
        for (_, _, part) in &parts {
            assert!(estimate_tokens(part) <= 128, "cap exceeded: {part:?}");
            assert!(
                part.trim_end().ends_with('。'),
                "sentence boundary expected, got {part:?}"
            );
        }
        for pair in parts.windows(2) {
            let previous = pair[0].2.trim_end_matches('。');
            let current = &pair[1].2;
            let shared = (1..=previous.chars().count().min(current.chars().count()))
                .rev()
                .find(|length| {
                    previous
                        .chars()
                        .skip(previous.chars().count() - length)
                        .eq(current.chars().take(*length))
                })
                .unwrap_or(0);
            assert!(
                shared > 0,
                "no overlap between {:?} and {current:?}",
                pair[0].2
            );
        }
    }

    #[test]
    fn structural_units_are_packed_until_they_no_longer_fit() {
        let text = "第一行。\n第二行。\n第三行。\n";
        let whole = split_block(text, 64, 8);
        assert_eq!(whole.len(), 1, "small units pack into one chunk");
        assert_eq!(whole[0].2, text);
        let split = split_block(text, 3, 1);
        assert!(split.len() > 1, "the cap still bounds the chunk size");
        contiguous(text, &split);
        for (_, _, part) in &split {
            assert!(estimate_tokens(part) <= 3);
        }
    }

    #[test]
    fn an_oversized_single_unit_is_hard_split_without_losing_text() {
        let text = "甲".repeat(500);
        let parts = split_block(&text, 64, 9);
        assert!(parts.len() > 1);
        contiguous(&text, &parts);
        for (_, _, part) in &parts {
            assert!(estimate_tokens(part) <= 64);
        }
        let ascii = "a".repeat(900);
        let parts = split_block(&ascii, 64, 9);
        contiguous(&ascii, &parts);
        for (_, _, part) in &parts {
            assert!(estimate_tokens(part) <= 64);
        }
    }

    #[test]
    fn token_estimate_counts_cjk_by_character_and_ascii_by_four_bytes() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("中文"), 2);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("中文abcd"), 3);
    }

    #[test]
    fn overlap_for_is_bounded_by_half_the_chunk() {
        assert_eq!(overlap_for(512, 15), 76);
        assert_eq!(overlap_for(512, 30), 153);
        assert_eq!(overlap_for(512, 99), 153, "percent is capped at 30");
        assert_eq!(overlap_for(64, 15), 9);
    }

    #[test]
    fn profile_identity_freezes_everything_but_the_chunk_cap() {
        let spec = default_profile_spec("https://example.invalid/v1".into());
        assert_eq!(spec.max_tokens, DEFAULT_CHUNK_TOKENS);
        assert_eq!(
            spec.overlap_tokens,
            overlap_for(DEFAULT_CHUNK_TOKENS, DEFAULT_OVERLAP_PERCENT)
        );
        assert!(valid(&profile()));
        let mut tampered = profile();
        tampered.spec.dimensions = 768;
        assert!(!valid(&tampered));
        let mut tampered = profile();
        tampered.spec.chunker = "pkms-utf8-boundary-v1:title-newline:no-overlap".into();
        assert!(!valid(&tampered));
        let mut smaller = profile();
        smaller.spec.max_tokens = 128;
        smaller.spec.overlap_tokens = overlap_for(128, 15);
        assert!(valid(&smaller), "the chunk cap is configurable");
        let mut too_big = profile();
        too_big.spec.max_tokens = MAX_CHUNK_TOKENS + 1;
        assert!(!valid(&too_big));
        let mut no_endpoint = profile();
        no_endpoint.spec.endpoint = None;
        assert!(!valid(&no_endpoint));
    }

    #[test]
    fn provider_model_and_output_size_are_part_of_the_profile_identity() {
        // The shipped default keeps the JSON that was registered before V1.7, so
        // an upgraded library does not rebuild an index that is still current.
        let legacy = RemoteProfile::cloudflare(
            "https://example.invalid/v1".into(),
            DEFAULT_CHUNK_TOKENS,
            overlap_for(DEFAULT_CHUNK_TOKENS, DEFAULT_OVERLAP_PERCENT),
        );
        let spec = legacy.spec();
        assert_eq!(spec.provider, None);
        assert_eq!(spec.output_dimensions, None);
        assert_eq!(spec.revision, REVISION);
        assert_eq!(
            spec,
            default_profile_spec("https://example.invalid/v1".into())
        );
        assert_eq!(
            remote(&EmbeddingProfile {
                id: "profile".into(),
                spec: spec.clone(),
            })
            .unwrap(),
            legacy
        );

        // Another provider, model or output size is another profile, and a
        // different profile is what forces the index rebuild.
        let mut openai = legacy.clone();
        openai.provider = EmbeddingProvider::Openai;
        openai.model = "text-embedding-3-small".into();
        openai.dimensions = 1536;
        assert_ne!(openai.spec(), spec);
        assert_eq!(openai.spec().provider, Some(EmbeddingProvider::Openai));
        let mut truncated = openai.clone();
        truncated.output_dimensions = Some(768);
        truncated.dimensions = 768;
        assert_ne!(truncated.spec(), openai.spec());
        assert_eq!(truncated.spec().output_dimensions, Some(768));

        // A stored profile whose requested size and measured size disagree is
        // refused instead of being mixed with the other vectors.
        let mut mismatched = truncated.spec();
        mismatched.dimensions = 1024;
        assert!(
            remote(&EmbeddingProfile {
                id: "profile".into(),
                spec: mismatched,
            })
            .is_none()
        );

        // A non-default endpoint stays valid: only the identity fields are frozen.
        let other = RemoteProfile::cloudflare("https://example.invalid/other".into(), 128, 19);
        assert!(valid(&EmbeddingProfile {
            id: "profile".into(),
            spec: other.spec(),
        }));
    }

    #[test]
    fn prepare_stays_inside_the_cap_and_covers_every_block() {
        let profile = profile();
        let long = "这一句用来验证分块上限与重叠。".repeat(120);
        let plan = prepare(
            &profile,
            input("分块验证", &[("block-1", &long), ("block-2", "短段落。")]),
        )
        .unwrap();
        assert!(plan.chunks.len() > 1);
        for chunk in &plan.chunks {
            assert!(chunk.token_count <= profile.spec.max_tokens, "{chunk:?}");
            assert!(!chunk.text.is_empty());
            assert!(chunk.chunk_id.chars().all(|c| c.is_ascii_alphanumeric()));
        }
        let mut covered = 0u32;
        for chunk in plan.chunks.iter().filter(|c| c.block_id == "block-1") {
            assert!(chunk.start_codepoint <= covered);
            covered = covered.max(chunk.end_codepoint);
        }
        assert_eq!(covered as usize, long.chars().count());
        let last = plan
            .chunks
            .iter()
            .find(|chunk| chunk.block_id == "block-2")
            .unwrap();
        assert_eq!(last.text, "短段落。");
        let again = prepare(
            &profile,
            input("分块验证", &[("block-1", &long), ("block-2", "短段落。")]),
        )
        .unwrap();
        assert_eq!(
            plan.chunks
                .iter()
                .map(|chunk| chunk.chunk_id.clone())
                .collect::<Vec<_>>(),
            again
                .chunks
                .iter()
                .map(|chunk| chunk.chunk_id.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_title_that_does_not_fit_the_cap_is_rejected() {
        let mut profile = profile();
        profile.spec.max_tokens = 64;
        profile.spec.overlap_tokens = 9;
        let long_title = "标".repeat(200);
        assert!(matches!(
            prepare(&profile, input(&long_title, &[("block", "正文")])),
            Err("embedding_title_token_limit")
        ));
    }

    #[test]
    fn unicode_spans_are_complete_and_round_trip() {
        let text = "中文😀abc\n".repeat(1000);
        let parts = spans(&text, 127).unwrap();
        assert_eq!(
            parts.iter().map(|(_, _, s)| s.as_str()).collect::<String>(),
            text
        );
        let mut end = 0;
        for (a, b, s) in parts {
            assert_eq!(a, end);
            assert!(s.len() <= 127);
            assert_eq!(b - a, s.chars().count() as u32);
            end = b;
        }
        assert_eq!(end, text.chars().count() as u32);
    }
}

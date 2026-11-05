//! Offline before/after comparison for the indexing chunker.
//!
//! The real semantic comparison runs against the configured embedding endpoint
//! (see `crates/pkms-app/tests/quality_retrieval.rs`, opt-in through
//! `PKMS_QUALITY_*`). This test compares the two chunkers without any network: it
//! measures how well a chunk covers the sentence an answer needs, whether chunk
//! ends land on structural boundaries, and how a plain BM25 ranking over the
//! chunks performs. Those numbers are a deterministic proxy, not a substitute for
//! the reserved-set run.
use pkms_inference::remote_chunks::{
    DEFAULT_CHUNK_TOKENS, DEFAULT_OVERLAP_PERCENT, estimate_tokens, overlap_for, split_block,
};
use std::collections::{BTreeMap, BTreeSet};

/// The chunker this replaces: per block, hard UTF-8 byte spans with no overlap.
const OLD_BYTE_BUDGET: usize = 2048;

fn old_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + OLD_BYTE_BUDGET).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(text[start..end].to_owned());
        start = end;
    }
    chunks
}

fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut ascii = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            ascii.push(character.to_ascii_lowercase());
            continue;
        }
        if !ascii.is_empty() {
            out.push(std::mem::take(&mut ascii));
        }
        if !character.is_ascii_whitespace() {
            out.push(character.to_string());
        }
    }
    if !ascii.is_empty() {
        out.push(ascii);
    }
    out
}

/// BM25 over chunks; returns the chunk index that wins for each query.
fn rank(chunks: &[String], queries: &[String], limit: usize) -> Vec<Vec<usize>> {
    let documents = chunks.iter().map(|chunk| tokens(chunk)).collect::<Vec<_>>();
    let lengths = documents.iter().map(|d| d.len()).collect::<Vec<_>>();
    let average = lengths.iter().sum::<usize>() as f64 / lengths.len().max(1) as f64;
    let mut document_frequency: BTreeMap<String, usize> = BTreeMap::new();
    for document in &documents {
        for term in document.iter().collect::<BTreeSet<_>>() {
            *document_frequency.entry(term.clone()).or_default() += 1;
        }
    }
    queries
        .iter()
        .map(|query| {
            let terms = tokens(query);
            let mut scored = documents
                .iter()
                .enumerate()
                .map(|(index, document)| {
                    let mut frequencies: BTreeMap<&str, usize> = BTreeMap::new();
                    for term in document {
                        *frequencies.entry(term.as_str()).or_default() += 1;
                    }
                    let mut score = 0.0;
                    for term in &terms {
                        let Some(frequency) = frequencies.get(term.as_str()) else {
                            continue;
                        };
                        let frequency = *frequency as f64;
                        let df = *document_frequency.get(term).unwrap_or(&1) as f64;
                        let idf = (1.0 + (documents.len() as f64 - df + 0.5) / (df + 0.5)).ln();
                        let length = lengths[index] as f64;
                        score += idf * (frequency * 2.2)
                            / (frequency + 1.2 * (1.0 - 0.75 + 0.75 * length / average.max(1.0)));
                    }
                    (index, score)
                })
                .collect::<Vec<_>>();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
            scored
                .into_iter()
                .take(limit)
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        })
        .collect()
}

struct Corpus {
    blocks: Vec<String>,
    answers: Vec<(String, String)>,
}

fn sentence(topic: &str, number: usize) -> String {
    format!("第{number:04}句关于{topic}的检索质量说明，用于触发分块。")
}

fn corpus() -> Corpus {
    let topics = [
        ("备份", "每晚九点执行完整备份并保留最近七天"),
        ("索引", "重建索引前必须冻结当前解析版本"),
        ("摘录", "摘录必须绑定原始资料的定位信息"),
        ("渠道", "渠道密钥保存在操作系统凭据库中"),
        ("清洗", "清洗只影响派生正文不影响原件"),
        ("分块", "分块边界优先落在段落与句子处"),
        ("导出", "导出包包含原件字节与清单哈希"),
        ("回顾", "回顾条目按到期时间排序展示"),
    ];
    let mut blocks = Vec::new();
    let mut answers = Vec::new();
    for (topic, answer) in topics {
        // Fill up to the old chunker's 2048-byte cut, then place the answer so that
        // the old hard cut lands inside it, then keep going so both chunkers split.
        let mut paragraph = String::new();
        let mut index = 0;
        while paragraph.len() < 2000 {
            paragraph.push_str(&sentence(topic, index));
            index += 1;
        }
        paragraph.push_str(&format!("{answer}。"));
        answers.push((topic.to_string(), format!("{answer}。")));
        while paragraph.len() < 3200 {
            paragraph.push_str(&sentence(topic, index));
            index += 1;
        }
        blocks.push(paragraph);
        // One much longer block, so both chunkers have to split more than once.
        let mut long = String::new();
        for index in 0..400 {
            long.push_str(&sentence(topic, index));
        }
        blocks.push(long);
    }
    Corpus { blocks, answers }
}
#[test]
fn structure_first_chunks_cover_answers_and_keep_boundaries() {
    let corpus = corpus();
    let mut old_all = Vec::new();
    let mut new_all = Vec::new();
    let mut old_contained = 0usize;
    let mut new_contained = 0usize;
    let mut old_boundary = 0usize;
    let mut new_boundary = 0usize;
    let mut old_ends = 0usize;
    let mut new_ends = 0usize;
    let mut old_max = 0u32;
    let mut new_max = 0u32;
    for block in &corpus.blocks {
        let old = old_chunks(block);
        let new = split_block(
            block,
            DEFAULT_CHUNK_TOKENS,
            overlap_for(DEFAULT_CHUNK_TOKENS, DEFAULT_OVERLAP_PERCENT),
        )
        .into_iter()
        .map(|(_, _, text)| text)
        .collect::<Vec<_>>();
        for (index, chunk) in old.iter().enumerate() {
            old_max = old_max.max(estimate_tokens(chunk));
            if index + 1 < old.len() {
                old_ends += 1;
                if chunk.trim_end().ends_with('。') || chunk.ends_with('\n') {
                    old_boundary += 1;
                }
            }
        }
        for (index, chunk) in new.iter().enumerate() {
            new_max = new_max.max(estimate_tokens(chunk));
            if index + 1 < new.len() {
                new_ends += 1;
                if chunk.trim_end().ends_with('。') || chunk.ends_with('\n') {
                    new_boundary += 1;
                }
            }
        }
        old_all.extend(old);
        new_all.extend(new);
    }
    for (_, answer) in &corpus.answers {
        if old_all.iter().any(|chunk| chunk.contains(answer)) {
            old_contained += 1;
        }
        if new_all.iter().any(|chunk| chunk.contains(answer)) {
            new_contained += 1;
        }
    }
    // A query is the answer sentence itself: the retriever has to return a chunk
    // that fully covers it within the top eight candidates.
    let queries = corpus
        .answers
        .iter()
        .map(|(_, answer)| answer.trim_end_matches('。').to_string())
        .collect::<Vec<_>>();
    let old_recall = rank(&old_all, &queries, 8);
    let new_recall = rank(&new_all, &queries, 8);
    let mut old_hits = 0usize;
    let mut new_hits = 0usize;
    for (index, (_, answer)) in corpus.answers.iter().enumerate() {
        if old_recall[index]
            .iter()
            .any(|candidate| old_all[*candidate].contains(answer))
        {
            old_hits += 1;
        }
        if new_recall[index]
            .iter()
            .any(|candidate| new_all[*candidate].contains(answer))
        {
            new_hits += 1;
        }
    }
    let total = corpus.answers.len();
    println!(
        "old: chunks {} max_tokens {} containment {}/{} boundary {}/{} recall@8 {}/{}",
        old_all.len(),
        old_max,
        old_contained,
        total,
        old_boundary,
        old_ends,
        old_hits,
        total
    );
    println!(
        "new: chunks {} max_tokens {} containment {}/{} boundary {}/{} recall@8 {}/{}",
        new_all.len(),
        new_max,
        new_contained,
        total,
        new_boundary,
        new_ends,
        new_hits,
        total
    );
    assert_eq!(
        new_contained, total,
        "every answer must fit inside one chunk"
    );
    assert!(
        new_max <= DEFAULT_CHUNK_TOKENS,
        "the new chunker must respect the token cap, got {new_max}"
    );
    assert!(
        new_contained >= old_contained,
        "answer containment must not regress"
    );
    assert!(
        new_boundary * old_ends >= old_boundary * new_ends,
        "structural boundaries must not regress ({new_boundary}/{new_ends} vs {old_boundary}/{old_ends})"
    );
    assert!(
        new_hits >= old_hits,
        "lexical recall@8 must not regress ({new_hits} vs {old_hits})"
    );
}

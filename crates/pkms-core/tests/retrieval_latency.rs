//! Local (network-free) retrieval latency baseline for the P2 rows of the SRS.
//!
//! The measured path is exactly the part of retrieval that never leaves the
//! device: query tokenization, FTS ranking, result fusion, evidence assembly and
//! the evidence freeze write. Embedding the query (one remote request) and the
//! remote vector scan are excluded, which is what the accepted V1.7 wording asks
//! for ("不含网络往返").
//!
//! Default size is 1,000 notes; set `PKMS_LATENCY_SCALE=10000` to run the ten
//! times larger corpus. Run with `--nocapture` to see the printed percentiles.
use pkms_core::*;
use std::time::Instant;
use tempfile::tempdir;
use uuid::Uuid;

const WARMUP: usize = 10;
const SAMPLES: usize = 50;
const TOPICS: [&str; 12] = [
    "备份", "索引", "权限", "导出", "标签", "回收", "检索", "同步", "批注", "模板", "统计", "范围",
];

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn corpus(size: usize) -> (tempfile::TempDir, Library) {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    for index in 0..size {
        let topic = TOPICS[index % TOPICS.len()];
        // Every tenth note carries a rare marker so queries stay selective.
        let marker = if index % 10 == 0 {
            format!("唯一标记{index}")
        } else {
            String::new()
        };
        let body = format!(
            "{topic}记录 {index}：{topic}流程需要保留七天，{topic}失败时按范围重试。\
             {topic}审批由负责人确认，{topic}结果写入台账并保留版本。{marker}其他说明文字用于凑长度。"
        );
        library
            .create_note(&format!("{topic}笔记 {index}"), &body)
            .unwrap();
    }
    (dir, library)
}

fn percentile(samples: &mut [f64], fraction: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    let index = ((samples.len() as f64 - 1.0) * fraction).ceil() as usize;
    samples[index.min(samples.len() - 1)]
}

fn report(label: &str, mut samples: Vec<f64>) -> (f64, f64) {
    let p50 = percentile(&mut samples, 0.50);
    let p95 = percentile(&mut samples, 0.95);
    let max = samples.last().copied().unwrap_or_default();
    println!(
        "{label}: p50={p50:.1}ms p95={p95:.1}ms max={max:.1}ms samples={}",
        samples.len()
    );
    (p50, p95)
}

#[test]
fn local_retrieval_latency_at_one_and_ten_thousand_notes() {
    let size: usize = std::env::var("PKMS_LATENCY_SCALE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000);
    assert!(
        matches!(size, 1_000 | 10_000),
        "PKMS_LATENCY_SCALE must be 1000 or 10000"
    );
    let started = Instant::now();
    let (_dir, mut library) = corpus(size);
    println!(
        "corpus={size} build={:.1}s",
        started.elapsed().as_secs_f64()
    );

    let mut search = Vec::with_capacity(SAMPLES + WARMUP);
    let mut freeze = Vec::with_capacity(SAMPLES + WARMUP);
    for sample in 0..SAMPLES + WARMUP {
        let topic = TOPICS[sample % TOPICS.len()];
        let query = SearchQuery {
            query: format!("{topic}保留几天？"),
            mode: SearchMode::Lexical,
            scope: SearchScope::default(),
            limit: 8,
            minimum_cosine: None,
        };
        let started = Instant::now();
        let results = library.keyword_search(&query).unwrap();
        let search_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert!(!results.items.is_empty(), "a topical query must match");

        // The same query through the full local evidence path: retrieval plus the
        // freeze write the answer pipeline performs before any model call.
        let accepted = library
            .accept_answer(
                &id(),
                StartAnswer {
                    max_output_tokens: None,
                    citation_count: Some(5),
                    agent: None,
                    search: query.clone(),
                    model: None,
                },
            )
            .unwrap();
        let lease = library.claim_answer(&accepted.answer.id).unwrap().unwrap();
        let started = Instant::now();
        library
            .prepare_answer_evidence(&accepted.answer.id, &lease.lease_token, vec![])
            .unwrap();
        let freeze_ms = started.elapsed().as_secs_f64() * 1000.0;

        if sample >= WARMUP {
            search.push(search_ms);
            freeze.push(freeze_ms);
        }
    }
    let (search_p50, search_p95) = report("local_search", search);
    let (freeze_p50, freeze_p95) = report("local_search_and_evidence_freeze", freeze);
    println!(
        "summary: size={size} search_p50={search_p50:.1} search_p95={search_p95:.1} \
         freeze_p50={freeze_p50:.1} freeze_p95={freeze_p95:.1}"
    );
    // Loose ceilings only: this test records a baseline and catches gross
    // regressions, it does not redefine the accepted performance wording.
    assert!(search_p95 < 2_000.0, "local search p95 {search_p95}");
    assert!(freeze_p95 < 4_000.0, "local freeze p95 {freeze_p95}");
}

#![cfg(feature = "embedding")]
use pkms_core::{EmbeddingProfile, EvidenceKind, EvidenceTarget, IndexInput, IndexInputBlock};
use pkms_inference::embedding::{Chunker, Embedder, profile_spec};
use std::path::PathBuf;

fn directory() -> PathBuf {
    std::env::var_os("PKMS_E5_MODEL_DIR")
        .map(PathBuf::from)
        .expect("set PKMS_E5_MODEL_DIR to verified model directory")
}
fn profile() -> EmbeddingProfile {
    EmbeddingProfile {
        id: "test-profile".into(),
        spec: profile_spec(),
    }
}
#[test]
fn missing_model_is_an_explicit_error_and_never_downloads() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        Chunker::load(directory.path()).err().unwrap().0,
        "embedding_model_unavailable"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    std::fs::write(directory.path().join("tokenizer.json"), b"{}").unwrap();
    assert_eq!(
        Chunker::load(directory.path()).err().unwrap().0,
        "embedding_model_hash_mismatch"
    );
}
#[test]
#[ignore = "requires pinned real tokenizer files"]
fn real_tokenizer_preserves_every_codepoint_and_accounts_for_title_prefix() {
    let chunker = Chunker::load(&directory()).unwrap();
    let text = format!(
        "  {}\n  ",
        "中文组合e\u{301} 😀 mixedCode::path 新知识与原始证据。".repeat(400)
    );
    let input = IndexInput {
        object_id: "object".into(),
        kind: EvidenceKind::Note,
        content_revision: "revision-1".into(),
        metadata_revision: 1,
        parser_revision: "test-v1".into(),
        title: "中文标题与title".into(),
        blocks: vec![IndexInputBlock {
            target: EvidenceTarget {
                object_id: "object".into(),
                kind: EvidenceKind::Note,
                content_revision: "revision-1".into(),
                metadata_revision: 1,
                parse_id: None,
                block_id: "block".into(),
                start_codepoint: 0,
                end_codepoint: text.chars().count() as u32,
                page_index: None,
            },
            text: text.clone(),
        }],
    };
    let plan = chunker.prepare(&profile(), &input).unwrap();
    assert!(plan.chunks.len() > 2);
    assert_eq!(
        plan.chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<String>(),
        text
    );
    let mut offset = 0;
    for chunk in &plan.chunks {
        assert_eq!(chunk.start_codepoint, offset);
        assert_eq!(
            chunk.end_codepoint - chunk.start_codepoint,
            chunk.text.chars().count() as u32
        );
        assert_eq!(
            chunk.token_count as usize,
            chunker
                .token_count(&format!("passage: {}\n{}", input.title, chunk.text))
                .unwrap()
        );
        assert!(chunk.token_count <= 512);
        offset = chunk.end_codepoint;
    }
    let again = chunker.prepare(&profile(), &input).unwrap();
    assert_eq!(plan.chunks[0].chunk_id, again.chunks[0].chunk_id);
}
#[test]
#[ignore = "requires pinned real ONNX runtime and model"]
fn real_onnx_embeddings_are_finite_normalized_and_reject_overbudget_inputs() {
    let loading = std::time::Instant::now();
    let mut model = Embedder::load(&directory(), &profile()).unwrap();
    eprintln!("model_load_ms={}", loading.elapsed().as_millis());
    let vectors = model
        .embed(vec![
            "query: 如何保留原始证据？".into(),
            "passage: 保存原件\n保留原始资料及不可变版本，引用必须能回查。".into(),
            "passage: 花园\n今天种植番茄并修剪花园里的玫瑰。".into(),
        ])
        .unwrap();
    assert_eq!(vectors.len(), 3);
    for vector in &vectors {
        assert_eq!(vector.len(), 384);
        assert!(vector.iter().all(|value| value.is_finite()));
        let norm = vector
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>();
        assert!((norm - 1.0).abs() < 1e-5);
    }
    let cosine = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .map(|(x, y)| f64::from(*x) * f64::from(*y))
            .sum::<f64>()
    };
    assert!(cosine(&vectors[0], &vectors[1]) > cosine(&vectors[0], &vectors[2]));
    let mut samples = Vec::new();
    for _ in 0..50 {
        let started = std::time::Instant::now();
        model
            .embed(vec!["query: 如何保留原始证据？".into()])
            .unwrap();
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    eprintln!("warm_query_embedding_ms={samples:?}");
    assert_eq!(
        model
            .embed(vec![format!(
                "query: {}",
                "原始证据需要被保留。".repeat(1000)
            )])
            .unwrap_err()
            .0,
        "embedding_token_budget"
    );
}

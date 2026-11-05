#![cfg(feature = "embedding")]
use pkms_core::EmbeddingProfile;
use pkms_inference::{
    embedding::{
        Chunker, Embedder, EmbeddingError, MAX_TOKENS, profile_spec, validate_exact_quote_request,
    },
    protocol::{ExactQuoteInput, Operation, WorkerResult},
};
use std::path::PathBuf;

fn profile() -> EmbeddingProfile {
    EmbeddingProfile {
        id: "exact-test-profile".into(),
        spec: profile_spec(),
    }
}
fn directory() -> PathBuf {
    std::env::var_os("PKMS_E5_MODEL_DIR")
        .map(PathBuf::from)
        .expect(
            "Set PKMS_E5_MODEL_DIR to the downloaded E5 model before running ignored model tests",
        )
}
fn input(text: &str) -> ExactQuoteInput {
    ExactQuoteInput {
        id: "quote".into(),
        text: text.into(),
    }
}

#[test]
fn exact_quote_request_rejects_ambiguous_ids_and_all_out_of_bounds_inputs() {
    assert!(validate_exact_quote_request("query", &[input("original")], 1).is_ok());
    for (query, inputs, limit) in [
        ("".into(), vec![input("original")], 1),
        ("q".repeat(4097), vec![input("original")], 1),
        ("query".into(), vec![], 1),
        ("query".into(), vec![input("original"); 129], 1),
        (
            "query".into(),
            vec![input("original"), input("different")],
            1,
        ),
        ("query".into(), vec![input(&"x".repeat(65537))], 1),
        ("query".into(), vec![input(" \n ")], 1),
        ("query".into(), vec![input("original")], 0),
        ("query".into(), vec![input("original")], 801),
    ] {
        assert!(validate_exact_quote_request(&query, &inputs, limit).is_err());
    }
    let oversized_id = ExactQuoteInput {
        id: "i".repeat(129),
        text: "original".into(),
    };
    assert!(validate_exact_quote_request("query", &[oversized_id], 1).is_err());
    let encoded = serde_json::to_value(Operation::ScoreExactQuotes {
        profile: profile(),
        query: "query".into(),
        inputs: vec![input("original")],
        limit: 1,
    })
    .unwrap();
    assert_eq!(encoded["kind"], "score_exact_quotes");
    assert!(
        serde_json::from_value::<ExactQuoteInput>(
            serde_json::json!({"id":"q","text":"quote","title":"forbidden context"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<WorkerResult>(
            serde_json::json!({"kind":"quote_scores","items":[]})
        )
        .is_ok()
    );
}

#[test]
#[ignore = "requires pinned E5 tokenizer files; no model inference"]
fn exact_quote_chunks_preserve_unicode_offsets_and_cover_without_truncation() {
    let chunker = Chunker::load(&directory()).unwrap();
    let text = format!("  {}\n ", "原始😀e\u{301} evidence 中文说明。".repeat(700));
    let inputs = vec![input(&text)];
    let chunks = chunker.prepare_exact_quotes(&profile(), &inputs).unwrap();
    assert!(chunks.len() > 1);
    let chars = text.chars().collect::<Vec<_>>();
    let mut end = 0;
    let mut rebuilt = String::new();
    for chunk in chunks {
        assert_eq!(chunk.id, "quote");
        assert_eq!(chunk.start_codepoint, end);
        assert!(chunk.start_codepoint < chunk.end_codepoint);
        assert_eq!(
            chunk.text,
            chars[chunk.start_codepoint as usize..chunk.end_codepoint as usize]
                .iter()
                .collect::<String>()
        );
        assert!(
            chunker
                .token_count(&format!("{}{}", profile().spec.passage_prefix, chunk.text))
                .unwrap()
                <= MAX_TOKENS
        );
        end = chunk.end_codepoint;
        rebuilt.push_str(&chunk.text);
    }
    assert_eq!(end as usize, chars.len());
    assert_eq!(rebuilt, text);
}

#[test]
#[ignore = "requires pinned real E5 model; computes actual embeddings and cosine"]
fn exact_quotes_score_their_own_text_and_allow_cancellation() {
    let mut embedder = Embedder::load(&directory(), &profile()).unwrap();
    let inputs = vec![
        ExactQuoteInput {
            id: "answer".into(),
            text: "The Kestrel cache capacity is 256 MiB.".into(),
        },
        ExactQuoteInput {
            id: "unrelated".into(),
            text: "Orchids require careful watering and indirect sunlight.".into(),
        },
    ];
    let query = "What is the capacity of the Kestrel cache?";
    let scores = embedder
        .score_exact_quotes(&profile(), query, &inputs, 2, &mut |_, _| Ok(()))
        .unwrap();
    assert_eq!(scores.len(), 2);
    assert_eq!(scores[0].id, "answer");
    assert!(scores[0].cosine_similarity >= scores[1].cosine_similarity);
    for score in &scores {
        let original = inputs.iter().find(|input| input.id == score.id).unwrap();
        assert_eq!(score.start_codepoint, 0);
        assert_eq!(score.end_codepoint as usize, original.text.chars().count());
        assert!(score.cosine_similarity.is_finite());
    }
    let top = embedder
        .score_exact_quotes(&profile(), query, &inputs, 1, &mut |_, _| Ok(()))
        .unwrap();
    assert_eq!(top[0].id, scores[0].id);
    assert_eq!(
        embedder
            .score_exact_quotes(&profile(), query, &inputs, 2, &mut |_, _| Err(
                EmbeddingError("worker_interrupted")
            ))
            .unwrap_err()
            .0,
        "worker_interrupted"
    );
}

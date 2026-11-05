//! Live provider calls against the real endpoints.
//!
//! These are ignored by default: they need real credentials and reach the public
//! internet, so they only run when asked for. The request itself goes through the
//! same `remote_embeddings::Client` the product uses, which is what makes this
//! evidence about the shipped transport rather than about a hand-written probe.
//!
//! ```text
//! $env:PKMS_LIVE_EMBED_BASE  = 'https://api.cloudflare.com/client/v4/accounts/<ACCOUNT_ID>/ai/v1'
//! $env:PKMS_LIVE_EMBED_KEY   = '<token>'
//! $env:PKMS_LIVE_EMBED_PROVIDER = 'cloudflare'
//! cargo test -p pkms-inference --features remote --test remote_live -- --ignored --nocapture
//! ```
//!
//! `PKMS_LIVE_EMBED_MODEL` and `PKMS_LIVE_EMBED_DIMENSIONS` are optional. Without
//! a dimension the call measures whatever the model returns, which is the same
//! probe the interface runs before saving a profile.
use pkms_inference::remote_embeddings::{Client, GeminiTask, MODEL, Wire};
use std::time::Duration;

fn environment(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn provider(base: &str, model: &str, dimensions: Option<u32>) -> Wire {
    match environment("PKMS_LIVE_EMBED_PROVIDER").as_deref() {
        Some("gemini") => Wire::Gemini {
            base: base.into(),
            model: model.into(),
            // gemini-embedding-2 rejects taskType; older models require it.
            task: (!model.contains("embedding-2")).then_some(GeminiTask::Document),
            dimensions,
        },
        Some("openai_compat") => Wire::OpenAi {
            base: base.into(),
            model: model.into(),
            dimensions,
            check_model: false,
        },
        Some("openai") => Wire::OpenAi {
            base: base.into(),
            model: model.into(),
            dimensions,
            check_model: true,
        },
        _ => {
            let mut wire = Wire::cloudflare(base);
            if let Wire::OpenAi {
                model: slot,
                dimensions: size,
                ..
            } = &mut wire
            {
                *slot = model.into();
                *size = dimensions;
            }
            wire
        }
    }
}

/// A profile that declares the wrong width must be refused by the client, not
/// stored and then fail at search time. This is the real-endpoint half of the
/// "dimension mismatch" requirement: the local half is covered by unit tests.
#[tokio::test]
#[ignore = "requires PKMS_LIVE_EMBED_BASE and PKMS_LIVE_EMBED_KEY"]
async fn a_declared_width_the_provider_does_not_return_is_rejected() {
    let (Some(base), Some(secret)) = (
        environment("PKMS_LIVE_EMBED_BASE"),
        environment("PKMS_LIVE_EMBED_KEY"),
    ) else {
        panic!("set PKMS_LIVE_EMBED_BASE and PKMS_LIVE_EMBED_KEY to run this test");
    };
    let model = environment("PKMS_LIVE_EMBED_MODEL").unwrap_or_else(|| MODEL.into());
    // The request asks for nothing extra: the mismatch is between what the caller
    // requires and what the provider actually returns, which is the case a saved
    // profile must never be allowed to reach.
    let wire = provider(&base, &model, None);
    let client = Client::new().expect("http client");
    let result = client
        .embed_wire(
            &wire,
            &secret,
            &["dimension mismatch probe".to_string()],
            Some(512),
            Duration::from_secs(60),
            || Ok(()),
        )
        .await;
    // `Embedded` is intentionally not Debug, so match rather than expect_err.
    let error = match result {
        Ok(_) => panic!("a wrong declared width must be refused"),
        Err(error) => error,
    };
    println!("live dimension mismatch rejected: {}", error);
}
#[tokio::test]
#[ignore = "requires PKMS_LIVE_EMBED_BASE and PKMS_LIVE_EMBED_KEY"]
async fn the_configured_provider_returns_usable_normalized_vectors() {
    let (Some(base), Some(secret)) = (
        environment("PKMS_LIVE_EMBED_BASE"),
        environment("PKMS_LIVE_EMBED_KEY"),
    ) else {
        panic!("set PKMS_LIVE_EMBED_BASE and PKMS_LIVE_EMBED_KEY to run this test");
    };
    let model = environment("PKMS_LIVE_EMBED_MODEL").unwrap_or_else(|| MODEL.into());
    // A model name does not carry its width, so an unset value measures whatever
    // the provider returns instead of asserting a guess.
    let expected =
        environment("PKMS_LIVE_EMBED_DIMENSIONS").and_then(|value| value.parse::<usize>().ok());
    let wire = provider(&base, &model, expected.map(|value| value as u32));

    let input = vec![
        "spanseek live probe: the knowledge index and the question vector use one model."
            .to_string(),
        "第二条探测文本：分块、重叠与引用。".to_string(),
    ];
    let client = Client::new().expect("http client");
    let started = std::time::Instant::now();
    let embedded = client
        .embed_wire(
            &wire,
            &secret,
            &input,
            expected,
            Duration::from_secs(60),
            || Ok(()),
        )
        .await
        .expect("live embedding call");
    let elapsed = started.elapsed();
    let vectors = embedded.vectors;

    assert_eq!(vectors.len(), input.len(), "one vector per input");
    let dimension = vectors[0].len();
    // The same ceiling the local projection accepts.\n    assert!(dimension > 0 && dimension <= 8192, "dimension {dimension}");
    if let Some(expected) = expected {
        assert_eq!(dimension, expected, "declared dimension");
    }
    for vector in &vectors {
        assert_eq!(vector.len(), dimension, "every vector shares one width");
        assert!(
            vector.iter().all(|value| value.is_finite()),
            "finite values"
        );
        let norm = vector
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(norm > 0.99 && norm < 1.01, "normalized vector, norm {norm}");
    }
    // Two different texts must not collapse to the same vector.
    let difference: f64 = vectors[0]
        .iter()
        .zip(&vectors[1])
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs())
        .sum();
    assert!(difference > 0.01, "distinct inputs produced one vector");
    println!(
        "live embedding ok: model={model} vectors={} dimension={dimension} elapsed_ms={}",
        vectors.len(),
        elapsed.as_millis()
    );
}

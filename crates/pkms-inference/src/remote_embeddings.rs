//! Bounded provider-native embedding transport. Every provider is called
//! through its own documented REST schema. Requests, responses and credentials
//! are never logged.
use reqwest::{Url, header, redirect::Policy};
use serde::Deserialize;
use std::time::{Duration, Instant};

/// Default model: Cloudflare Workers AI BGE-M3.
pub const MODEL: &str = "@cf/baai/bge-m3";
pub const DIMENSIONS: usize = 1024;
pub const MAX_BATCH: usize = 32;
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 128 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub &'static str);

/// Plain HTTP is only acceptable for an embedding server running on this machine
/// (Ollama, LM Studio, vLLM, TEI). Anything reachable over the network is HTTPS.
pub fn loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn base_url(value: &str) -> Result<Url, Error> {
    let mut url = Url::parse(value).map_err(|_| Error("embedding_invalid_endpoint"))?;
    let scheme_ok =
        url.scheme() == "https" || (url.scheme() == "http" && url.host_str().is_some_and(loopback));
    if !scheme_ok
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(Error("embedding_invalid_endpoint"));
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url)
}

/// OpenAI-shaped endpoint: `{base}/embeddings`. A base without a path still
/// resolves to a single-slash path; `//embeddings` is a different URL that most
/// servers answer with 404.
pub fn endpoint(value: &str) -> Result<Url, Error> {
    let mut url = base_url(value)?;
    let base = url.path().trim_end_matches('/').to_owned();
    url.set_path(&format!("{base}/embeddings"));
    Ok(url)
}

/// Gemini endpoint: `{base}/models/{model}:batchEmbedContents`.
pub fn gemini_endpoint(value: &str, model: &str) -> Result<Url, Error> {
    let model = model.strip_prefix("models/").unwrap_or(model);
    if model.is_empty()
        || model.len() > 1024
        || model
            .chars()
            .any(|c| c.is_control() || c == '/' || c == ':')
    {
        return Err(Error("embedding_invalid_endpoint"));
    }
    let mut url = base_url(value)?;
    let base = url.path().trim_end_matches('/').to_owned();
    url.set_path(&format!("{base}/models/{model}:batchEmbedContents"));
    Ok(url)
}

/// Gemini distinguishes indexing from querying; the task is part of the request
/// contract and changes the embedding, so it is never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiTask {
    Document,
    Query,
}
impl GeminiTask {
    fn as_str(self) -> &'static str {
        match self {
            Self::Document => "RETRIEVAL_DOCUMENT",
            Self::Query => "RETRIEVAL_QUERY",
        }
    }
}

/// One provider request shape, including its documented field names.
#[derive(Debug, Clone)]
pub enum Wire {
    /// OpenAI-shaped `/embeddings`: OpenAI itself, Cloudflare's documented
    /// OpenAI-compatible Workers AI endpoint, and any other compatible server.
    OpenAi {
        base: String,
        model: String,
        /// Sent only when the user asked for a truncated output size.
        dimensions: Option<u32>,
        /// OpenAI and Cloudflare echo the model in the response; other
        /// compatible servers are not required to, so the check is optional.
        check_model: bool,
    },
    /// Gemini `models/{model}:batchEmbedContents`.
    Gemini {
        base: String,
        model: String,
        /// `None` for models that reject `taskType` (gemini-embedding-2).
        task: Option<GeminiTask>,
        dimensions: Option<u32>,
    },
}

impl Wire {
    /// The default Cloudflare-compatible request used by the shipped profile.
    pub fn cloudflare(base: &str) -> Self {
        Self::OpenAi {
            base: base.into(),
            model: MODEL.into(),
            dimensions: None,
            check_model: true,
        }
    }
    pub fn url(&self) -> Result<Url, Error> {
        match self {
            Self::OpenAi { base, .. } => endpoint(base),
            Self::Gemini { base, model, .. } => gemini_endpoint(base, model),
        }
    }
    fn body(&self, input: &[String]) -> serde_json::Value {
        match self {
            Self::OpenAi {
                model, dimensions, ..
            } => {
                let mut body = serde_json::json!({
                    "model": model,
                    "input": input,
                    "encoding_format": "float",
                });
                if let Some(value) = dimensions {
                    body["dimensions"] = serde_json::Value::from(*value);
                }
                body
            }
            Self::Gemini {
                model,
                task,
                dimensions,
                ..
            } => {
                let model = format!("models/{}", model.strip_prefix("models/").unwrap_or(model));
                let mut config = serde_json::Map::new();
                if let Some(task) = task {
                    config.insert("taskType".into(), task.as_str().into());
                }
                if let Some(value) = dimensions {
                    config.insert("outputDimensionality".into(), (*value).into());
                }
                // Providers truncate silently only when told to; a longer input
                // than the model window must fail loudly instead of being cut.
                config.insert("autoTruncate".into(), false.into());
                let requests: Vec<serde_json::Value> = input
                    .iter()
                    .map(|text| {
                        serde_json::json!({
                            "model": model,
                            "content": {"parts": [{"text": text}]},
                            "embedContentConfig": serde_json::Value::Object(config.clone()),
                        })
                    })
                    .collect();
                serde_json::json!({"requests": requests})
            }
        }
    }
    fn authorization(
        &self,
        secret: &str,
    ) -> Result<(header::HeaderName, header::HeaderValue), Error> {
        if secret.is_empty() || secret.len() > 2560 || !secret.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(Error("model_credential_unavailable"));
        }
        let (name, value) = match self {
            Self::OpenAi { .. } => (header::AUTHORIZATION, format!("Bearer {secret}")),
            Self::Gemini { .. } => (
                header::HeaderName::from_static("x-goog-api-key"),
                secret.to_owned(),
            ),
        };
        let value = header::HeaderValue::from_str(&value)
            .map_err(|_| Error("model_credential_unavailable"))?;
        let mut value = value;
        value.set_sensitive(true);
        Ok((name, value))
    }
    pub fn decode(
        &self,
        bytes: &[u8],
        count: usize,
        dimensions: Option<usize>,
    ) -> Result<Embedded, Error> {
        match self {
            Self::OpenAi {
                model, check_model, ..
            } => decode_openai(
                bytes,
                count,
                dimensions,
                check_model.then_some(model.as_str()),
            ),
            Self::Gemini { .. } => decode_gemini(bytes, count, dimensions),
        }
    }
}

/// Vectors plus the provider-reported input usage, when the provider returns it.
pub struct Embedded {
    pub vectors: Vec<Vec<f32>>,
    pub prompt_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    data: Vec<OpenAiItem>,
    model: Option<String>,
    usage: Option<OpenAiUsage>,
}
#[derive(Deserialize)]
struct OpenAiItem {
    index: usize,
    embedding: Vec<f32>,
}
#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<u64>,
}
#[derive(Deserialize)]
struct GeminiResponse {
    embeddings: Vec<GeminiEmbedding>,
    #[serde(rename = "usageMetadata")]
    usage: Option<GeminiUsage>,
}
#[derive(Deserialize)]
struct GeminiEmbedding {
    values: Vec<f32>,
}
#[derive(Deserialize)]
struct GeminiUsage {
    #[serde(rename = "promptTokenCount")]
    prompt_tokens: Option<u64>,
}

fn normalize(vector: Vec<f32>, dimensions: Option<usize>) -> Result<Vec<f32>, Error> {
    if vector.is_empty()
        || dimensions.is_some_and(|expected| vector.len() != expected)
        || vector.iter().any(|value| !value.is_finite())
    {
        return Err(Error("embedding_invalid_vector"));
    }
    let norm = vector
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt();
    if norm == 0. || !norm.is_finite() {
        return Err(Error("embedding_invalid_vector"));
    }
    Ok(vector
        .into_iter()
        .map(|value| (f64::from(value) / norm) as f32)
        .collect())
}

fn assemble(
    count: usize,
    dimensions: Option<usize>,
    rows: Vec<(usize, Vec<f32>)>,
) -> Result<Vec<Vec<f32>>, Error> {
    if rows.len() != count {
        return Err(Error("embedding_invalid_vector_count"));
    }
    let mut output = vec![None; count];
    for (index, vector) in rows {
        if index >= count || output[index].is_some() {
            return Err(Error("embedding_invalid_vector"));
        }
        output[index] = Some(normalize(vector, dimensions)?);
    }
    output
        .into_iter()
        .map(|value| value.ok_or(Error("embedding_invalid_vector_count")))
        .collect()
}

/// OpenAI-shaped response. `model` pins the response to the requested model when
/// the provider is documented to echo it.
pub fn decode_openai(
    bytes: &[u8],
    count: usize,
    dimensions: Option<usize>,
    model: Option<&str>,
) -> Result<Embedded, Error> {
    let response: OpenAiResponse =
        serde_json::from_slice(bytes).map_err(|_| Error("embedding_invalid_response"))?;
    // A response for a different model would silently mix embedding spaces, so
    // it is rejected with the existing vector-count code.
    if model.is_some_and(|expected| {
        response
            .model
            .as_deref()
            .is_some_and(|actual| actual != expected)
    }) {
        return Err(Error("embedding_invalid_vector_count"));
    }
    let rows = response
        .data
        .into_iter()
        .map(|item| (item.index, item.embedding))
        .collect();
    Ok(Embedded {
        vectors: assemble(count, dimensions, rows)?,
        prompt_tokens: response.usage.and_then(|usage| usage.prompt_tokens),
    })
}

/// Gemini `BatchEmbedContentsResponse`: one entry per request, in request order,
/// plus the prompt token usage used for cost accounting.
pub fn decode_gemini(
    bytes: &[u8],
    count: usize,
    dimensions: Option<usize>,
) -> Result<Embedded, Error> {
    let response: GeminiResponse =
        serde_json::from_slice(bytes).map_err(|_| Error("embedding_invalid_response"))?;
    let rows = response
        .embeddings
        .into_iter()
        .enumerate()
        .map(|(index, embedding)| (index, embedding.values))
        .collect();
    Ok(Embedded {
        vectors: assemble(count, dimensions, rows)?,
        prompt_tokens: response.usage.and_then(|usage| usage.prompt_tokens),
    })
}

/// The default Cloudflare response decoder.
pub fn decode(bytes: &[u8], count: usize) -> Result<Vec<Vec<f32>>, Error> {
    Ok(decode_openai(bytes, count, Some(DIMENSIONS), Some(MODEL))?.vectors)
}

/// Reusable HTTP pool. Credentials are supplied per request, never stored as defaults.
pub struct Client {
    http: reqwest::Client,
}
impl Client {
    pub fn new() -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(2)
            .build()
            .map_err(|_| Error("embedding_unavailable"))?;
        Ok(Self { http })
    }
    pub async fn embed(
        &self,
        base: &str,
        secret: &str,
        input: &[String],
        timeout: Duration,
        check: impl FnMut() -> Result<(), Error>,
    ) -> Result<Vec<Vec<f32>>, Error> {
        let wire = Wire::cloudflare(base);
        Ok(self
            .embed_wire(&wire, secret, input, Some(DIMENSIONS), timeout, check)
            .await?
            .vectors)
    }
    /// Provider-native call. `expected_dimensions` is the vector size the caller
    /// requires; `None` measures whatever the model returns, which is what the
    /// save-time probe does before a profile exists.
    pub async fn embed_wire(
        &self,
        wire: &Wire,
        secret: &str,
        input: &[String],
        expected_dimensions: Option<usize>,
        timeout: Duration,
        check: impl FnMut() -> Result<(), Error>,
    ) -> Result<Embedded, Error> {
        let call = Call {
            url: wire.url()?,
            wire,
            secret,
            input,
            expected_dimensions,
            timeout,
        };
        self.send(call, check).await
    }
    // The private entry point also lets transport tests exercise real sockets
    // without weakening the public HTTPS boundary.
    async fn send(
        &self,
        call: Call<'_>,
        mut check: impl FnMut() -> Result<(), Error>,
    ) -> Result<Embedded, Error> {
        let Call {
            wire,
            url,
            secret,
            input,
            expected_dimensions,
            timeout,
        } = call;
        if input.is_empty()
            || input.len() > MAX_BATCH
            || input
                .iter()
                .any(|text| text.trim().is_empty() || text.len() > 8192)
            || input.iter().map(String::len).sum::<usize>() > MAX_INPUT_BYTES
        {
            return Err(Error("embedding_input_limit"));
        }
        let (name, value) = wire.authorization(secret)?;
        let body = wire.body(input);
        let started = Instant::now();
        let mut poll = || {
            check()?;
            if started.elapsed() >= timeout {
                return Err(Error("inference_timeout"));
            }
            Ok(())
        };
        poll()?;
        let send = self.http.post(url).header(name, value).json(&body).send();
        tokio::pin!(send);
        let mut response = loop {
            poll()?;
            tokio::select! { result=&mut send => break result.map_err(|_| Error("embedding_unavailable"))?,
            _=tokio::time::sleep(Duration::from_millis(50)) => {} }
        };
        match response.status().as_u16() {
            200 => {}
            401 | 403 => return Err(Error("remote_authentication_failed")),
            404 => return Err(Error("embedding_model_unavailable")),
            429 => return Err(Error("remote_rate_limited")),
            _ => return Err(Error("embedding_api_failed")),
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE as u64)
        {
            return Err(Error("embedding_response_limit"));
        }
        let mut bytes = Vec::new();
        loop {
            let read = response.chunk();
            tokio::pin!(read);
            let chunk = loop {
                poll()?;
                tokio::select! {
                value=&mut read => break value.map_err(|_| Error("embedding_invalid_response"))?,
                _=tokio::time::sleep(Duration::from_millis(50)) => {} }
            };
            let Some(chunk) = chunk else {
                break;
            };
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err(Error("embedding_response_limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        poll()?;
        wire.decode(&bytes, input.len(), expected_dimensions)
    }
    #[cfg(test)]
    async fn embed_at(
        &self,
        url: Url,
        secret: &str,
        input: &[String],
        timeout: Duration,
        check: impl FnMut() -> Result<(), Error>,
    ) -> Result<Vec<Vec<f32>>, Error> {
        let wire = Wire::cloudflare("https://example.invalid/ai/v1");
        let call = Call {
            url,
            wire: &wire,
            secret,
            input,
            expected_dimensions: Some(DIMENSIONS),
            timeout,
        };
        Ok(self.send(call, check).await?.vectors)
    }
}

/// One transport call: everything about the request except its cancellation.
struct Call<'a> {
    wire: &'a Wire,
    url: Url,
    secret: &'a str,
    input: &'a [String],
    expected_dimensions: Option<usize>,
    timeout: Duration,
}
/// One-shot compatibility entry point. Long-lived callers should reuse `Client`.
pub async fn embed(
    base: &str,
    secret: &str,
    input: &[String],
    timeout: Duration,
    check: impl FnMut() -> Result<(), Error>,
) -> Result<Vec<Vec<f32>>, Error> {
    Client::new()?
        .embed(base, secret, input, timeout, check)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn response_order_dimension_count_and_norm_are_enforced() {
        let vector = vec![1.; DIMENSIONS];
        let bytes=serde_json::to_vec(&json!({"model":MODEL,"data":[{"index":1,"embedding":vector},{"index":0,"embedding":vec![2.;DIMENSIONS]}]})).unwrap();
        let result = decode(&bytes, 2).unwrap();
        assert!((result[0][0] - 0.03125).abs() < 1e-6);
        for data in [
            json!([{ "index":0,"embedding":vector},{"index":0,"embedding":vector}]),
            json!([{ "index":2,"embedding":vector}]),
            json!([{ "index":0,"embedding":[1.]}]),
            json!([{ "index":0,"embedding":vec![0.;DIMENSIONS]}]),
        ] {
            assert!(decode(&serde_json::to_vec(&json!({"data":data})).unwrap(), 1).is_err());
        }
        assert!(decode(&bytes, 1).is_err());
    }
    #[test]
    fn endpoints_preserve_account_path_and_reject_credentials() {
        assert_eq!(
            endpoint("https://example.com/accounts/a/ai/v1/")
                .unwrap()
                .as_str(),
            "https://example.com/accounts/a/ai/v1/embeddings"
        );
        for value in [
            "http://example.com",
            "https://u:p@example.com",
            "https://example.com?x=1",
            "https://example.com/#x",
        ] {
            assert!(endpoint(value).is_err());
        }
    }
    #[test]
    fn provider_bodies_use_each_documented_field_name() {
        let openai = Wire::OpenAi {
            base: "https://api.openai.com/v1".into(),
            model: "text-embedding-3-small".into(),
            dimensions: None,
            check_model: true,
        };
        let body = openai.body(&["alpha".into()]);
        assert_eq!(body["model"], json!("text-embedding-3-small"));
        assert_eq!(body["input"], json!(["alpha"]));
        assert_eq!(body["encoding_format"], json!("float"));
        assert!(body.get("dimensions").is_none());
        let truncated = Wire::OpenAi {
            base: "https://api.openai.com/v1".into(),
            model: "text-embedding-3-small".into(),
            dimensions: Some(768),
            check_model: true,
        };
        assert_eq!(truncated.body(&["alpha".into()])["dimensions"], json!(768));

        let gemini = Wire::Gemini {
            base: "https://generativelanguage.googleapis.com/v1beta".into(),
            model: "models/gemini-embedding-001".into(),
            task: Some(GeminiTask::Document),
            dimensions: Some(768),
        };
        assert_eq!(
            gemini.url().unwrap().as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:batchEmbedContents"
        );
        let body = gemini.body(&["alpha".into(), "beta".into()]);
        let requests = body["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["model"], json!("models/gemini-embedding-001"));
        assert_eq!(requests[0]["content"]["parts"][0]["text"], json!("alpha"));
        assert_eq!(
            requests[1]["embedContentConfig"]["taskType"],
            json!("RETRIEVAL_DOCUMENT")
        );
        assert_eq!(
            requests[0]["embedContentConfig"]["outputDimensionality"],
            json!(768)
        );
        assert_eq!(
            requests[0]["embedContentConfig"]["autoTruncate"],
            json!(false)
        );
        // gemini-embedding-2 rejects taskType, so the field must be absent there.
        let untasked = Wire::Gemini {
            base: "https://generativelanguage.googleapis.com/v1beta".into(),
            model: "gemini-embedding-2".into(),
            task: None,
            dimensions: None,
        };
        let body = untasked.body(&["alpha".into()]);
        assert!(
            body["requests"][0]["embedContentConfig"]
                .get("taskType")
                .is_none()
        );
        assert!(
            untasked.body(&["alpha".into()])["requests"][0]["embedContentConfig"]
                .get("outputDimensionality")
                .is_none()
        );
    }
    #[test]
    fn gemini_responses_keep_request_order_and_report_usage() {
        let bytes = serde_json::to_vec(&json!({
            "embeddings": [
                {"values": [1.0, 0.0, 0.0, 0.0]},
                {"values": [3.0, 4.0, 0.0, 0.0]}
            ],
            "usageMetadata": {"promptTokenCount": 7}
        }))
        .unwrap();
        let decoded = decode_gemini(&bytes, 2, Some(4)).unwrap();
        assert_eq!(decoded.vectors.len(), 2);
        assert!((decoded.vectors[0][0] - 1.0).abs() < 1e-6);
        assert!((decoded.vectors[1][0] - 0.6).abs() < 1e-6);
        assert!((decoded.vectors[1][1] - 0.8).abs() < 1e-6);
        assert_eq!(decoded.prompt_tokens, Some(7));
        assert!(decode_gemini(&bytes, 1, Some(4)).is_err());
        assert!(decode_gemini(&bytes, 2, Some(8)).is_err());
        assert!(decode_gemini(&bytes, 2, Some(4)).is_ok());
        assert!(decode_gemini(b"not json", 2, Some(4)).is_err());
    }
    #[test]
    fn openai_responses_report_usage_and_reject_a_different_model() {
        let bytes = serde_json::to_vec(&json!({
            "model": "text-embedding-3-small",
            "data": [{"index": 0, "embedding": vec![1.0; 4]}],
            "usage": {"prompt_tokens": 11, "total_tokens": 11}
        }))
        .unwrap();
        let decoded = decode_openai(&bytes, 1, Some(4), Some("text-embedding-3-small")).unwrap();
        assert_eq!(decoded.prompt_tokens, Some(11));
        assert!(decode_openai(&bytes, 1, Some(4), Some("other-model")).is_err());
        // A compatible server is not required to echo the model at all.
        assert!(decode_openai(&bytes, 1, Some(4), None).is_ok());
    }
    #[test]
    fn endpoints_allow_loopback_http_only() {
        assert_eq!(
            endpoint("http://127.0.0.1:11434/v1").unwrap().as_str(),
            "http://127.0.0.1:11434/v1/embeddings"
        );
        assert_eq!(
            endpoint("http://localhost:1234/v1/").unwrap().as_str(),
            "http://localhost:1234/v1/embeddings"
        );
        assert!(endpoint("http://example.com/v1").is_err());
        assert!(endpoint("http://192.168.1.5:11434/v1").is_err());
        assert!(gemini_endpoint("https://example.com/v1beta", "a/b").is_err());
        assert!(gemini_endpoint("https://example.com/v1beta", "").is_err());
    }
    #[tokio::test]
    async fn canceled_before_send_never_issues_request() {
        let result = embed(
            "https://example.invalid/ai/v1",
            "synthetic-token",
            &["synthetic".into()],
            Duration::from_secs(10),
            || Err(Error("canceled")),
        )
        .await;
        assert_eq!(result.unwrap_err().0, "canceled");
    }
}

#[cfg(test)]
#[path = "remote_embeddings_transport_tests.rs"]
mod transport_tests;

use super::{parser::Parser, sse::Sse, *};
use reqwest::{Url, header};
use std::net::IpAddr;
use tokio::time::Instant;
#[cfg(test)]
mod channel_tests;

pub struct Client {
    http: reqwest::Client,
    protocol: Protocol,
    url: Url,
    base: String,
    limits: Limits,
    transport: GenerationTransport,
    channel_id: Option<String>,
}
impl Client {
    /// `api_base_url` includes its version/prefix, e.g. https://provider.example/custom/v1.
    /// No implicit /v1, query credentials, redirects, environment proxies, or retries.
    pub fn new(protocol: Protocol, api_base_url: &str, limits: Limits) -> Result<Self, Error> {
        Self::new_with_transport(protocol, api_base_url, limits, GenerationTransport::Stream)
    }
    pub fn new_with_transport(
        protocol: Protocol,
        api_base_url: &str,
        limits: Limits,
        transport: GenerationTransport,
    ) -> Result<Self, Error> {
        let (base, url) = endpoint(protocol, api_base_url)?;
        let loopback_http = url.scheme() == "http"
            && url
                .host_str()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .is_some_and(|address| address.is_loopback());
        if limits.total_timeout.is_zero()
            || limits.first_event_timeout.is_zero()
            || limits.idle_timeout.is_zero()
            || limits.max_event_bytes == 0
            || limits.max_event_bytes > limits.max_response_bytes
            || limits.max_response_bytes > 32 * 1024 * 1024
            || !(1..=32).contains(&limits.max_tool_calls)
        {
            return Err(Error::InvalidRequest);
        }
        let http = reqwest::Client::builder()
            .https_only(!loopback_http)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| Error::Unavailable)?;
        Ok(Self {
            http,
            protocol,
            url,
            base,
            limits,
            transport,
            channel_id: None,
        })
    }
    /// Scope replay to the saved channel snapshot, not another account using the same URL.
    pub fn with_channel_id(mut self, channel_id: Option<&str>) -> Result<Self, Error> {
        if channel_id.is_some_and(|id| !request::identifier(id)) {
            return Err(Error::InvalidRequest);
        }
        self.channel_id = channel_id.map(str::to_owned);
        Ok(self)
    }
    /// Each callback is synchronous and must return promptly. Cancellation is checked at
    /// least every 50 ms while awaiting HTTP, and between SSE frames. No automatic retry.
    /// App must require Finish::Stop for final answers and validate their evidence schema.
    pub async fn generate(
        &self,
        request: &Request,
        key: &ApiKey,
        mut on_event: impl FnMut(Event) -> Result<(), Error>,
        mut canceled: impl FnMut() -> bool,
    ) -> Result<Completion, Error> {
        if canceled() {
            return Err(Error::Canceled);
        }
        let body = request::wire_with_options(
            self.protocol,
            &self.base,
            self.channel_id.as_deref(),
            self.transport,
            request,
        )?;
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static(match self.transport {
                GenerationTransport::Stream => "text/event-stream",
                GenerationTransport::Json => "application/json",
            }),
        );
        match self.protocol {
            Protocol::ChatCompletions | Protocol::Responses => {
                let raw = key.0.to_str().map_err(|_| Error::InvalidKey)?;
                let mut value = header::HeaderValue::from_str(&format!("Bearer {raw}"))
                    .map_err(|_| Error::InvalidKey)?;
                value.set_sensitive(true);
                headers.insert(header::AUTHORIZATION, value);
            }
            Protocol::AnthropicMessages => {
                headers.insert("x-api-key", key.0.clone());
                headers.insert(
                    "anthropic-version",
                    header::HeaderValue::from_static("2023-06-01"),
                );
            }
            Protocol::Gemini => {
                headers.insert("x-goog-api-key", key.0.clone());
            }
        }
        let started = Instant::now();
        let mut progress = None;
        let send = self
            .http
            .post(if self.protocol == Protocol::Gemini {
                super::gemini::url(&self.base, &request.model, self.transport)?
            } else {
                self.url.clone()
            })
            .headers(headers)
            .json(&body)
            .send();
        tokio::pin!(send);
        let mut response = loop {
            check(self.limits, started, progress, &mut canceled)?;
            tokio::select! { result = &mut send => break result.map_err(|_| Error::Unavailable)?, _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
        };
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(Error::Authentication),
            429 => return Err(Error::RateLimited),
            400..=499 => return Err(Error::RequestRejected),
            _ => return Err(Error::ApiError),
        }
        if response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .is_none_or(|value| {
                let expected = match self.transport {
                    GenerationTransport::Stream => "text/event-stream",
                    GenerationTransport::Json => "application/json",
                };
                !value.trim().eq_ignore_ascii_case(expected)
            })
        {
            return Err(Error::InvalidResponse);
        }
        if response
            .content_length()
            .is_some_and(|n| n > self.limits.max_response_bytes as u64)
        {
            return Err(Error::ResponseTooLarge);
        }
        let mut sse = Sse::new(self.limits.max_event_bytes);
        let mut parser = Parser::new(self.protocol, self.limits.max_tool_calls);
        let mut total = 0usize;
        let mut json_bytes = Vec::new();
        loop {
            let pending = response.chunk();
            tokio::pin!(pending);
            let chunk = loop {
                check(self.limits, started, progress, &mut canceled)?;
                tokio::select! { result = &mut pending => break result.map_err(|_| Error::Incomplete)?, _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
            };
            let Some(chunk) = chunk else {
                check(self.limits, started, progress, &mut canceled)?;
                if self.transport == GenerationTransport::Json {
                    let value =
                        serde_json::from_slice(&json_bytes).map_err(|_| Error::InvalidResponse)?;
                    parser.json(&value, &mut on_event)?;
                } else {
                    if !sse.is_empty() {
                        return Err(Error::Incomplete);
                    }
                    parser.end()?;
                }
                check(self.limits, started, progress, &mut canceled)?;
                return self.complete(parser, request);
            };
            total = total
                .checked_add(chunk.len())
                .ok_or(Error::ResponseTooLarge)?;
            if total > self.limits.max_response_bytes {
                return Err(Error::ResponseTooLarge);
            }
            if self.transport == GenerationTransport::Json {
                // JSON has no semantic events before the document completes. Its first/idle
                // budget tracks non-whitespace body progress; no bytes are an answer yet.
                if chunk.iter().any(|byte| !byte.is_ascii_whitespace()) {
                    progress = Some(Instant::now());
                }
                json_bytes.extend_from_slice(&chunk);
                continue;
            }
            sse.push(&chunk, |frame| {
                check(self.limits, started, progress, &mut canceled)?;
                let before = parser.activity();
                parser.frame(frame, &mut on_event)?;
                if parser.activity() != before {
                    progress = Some(Instant::now());
                }
                Ok(())
            })?;
            check(self.limits, started, progress, &mut canceled)?;
            if parser.done() {
                if !sse.is_empty() {
                    return Err(Error::InvalidResponse);
                }
                return self.complete(parser, request);
            }
        }
    }
    fn complete(&self, parser: Parser, request: &Request) -> Result<Completion, Error> {
        let mut result = parser.complete(&self.base, &request.model)?;
        // Provisional arguments cannot become executable without a declared function.
        if result
            .tool_calls
            .iter()
            .any(|call| !request.tools.iter().any(|tool| tool.name == call.name))
        {
            return Err(Error::UnsupportedOutput);
        }
        result.continuation.channel_id = self.channel_id.clone();
        Ok(result)
    }
}
fn check(
    limits: Limits,
    started: Instant,
    progress: Option<Instant>,
    canceled: &mut impl FnMut() -> bool,
) -> Result<(), Error> {
    if canceled() {
        return Err(Error::Canceled);
    }
    if started.elapsed() >= limits.total_timeout {
        return Err(Error::TotalTimeout);
    }
    match progress {
        None if started.elapsed() >= limits.first_event_timeout => Err(Error::FirstEventTimeout),
        Some(last) if last.elapsed() >= limits.idle_timeout => Err(Error::IdleTimeout),
        _ => Ok(()),
    }
}
fn endpoint(protocol: Protocol, base: &str) -> Result<(String, Url), Error> {
    if base.len() > 2048
        || base.chars().any(|c| c.is_control() || c.is_whitespace())
        || base.contains('\\')
    {
        return Err(Error::InvalidEndpoint);
    }
    let mut url = Url::parse(base).map_err(|_| Error::InvalidEndpoint)?;
    let loopback_http = url.scheme() == "http"
        && url
            .host_str()
            .and_then(|host| host.parse::<IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
    if (url.scheme() != "https" && !loopback_http)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidEndpoint);
    }
    let base = url.as_str().trim_end_matches('/').to_owned();
    let suffix = match protocol {
        Protocol::ChatCompletions => "chat/completions",
        Protocol::Responses => "responses",
        Protocol::AnthropicMessages => "messages",
        Protocol::Gemini => "",
    };
    if !suffix.is_empty() {
        url = Url::parse(&format!("{base}/{suffix}")).map_err(|_| Error::InvalidEndpoint)?;
    }
    Ok((base, url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
    };
    // A test-only local HTTP peer exercises transport without external API traffic or TLS
    // bypass in the production constructor. It uses only synthetic prompts/credentials.
    fn spawn_peer(
        protocol: Protocol,
        response: String,
        delay: Duration,
        limits: Limits,
    ) -> (Client, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (send, receive) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 16384);
            }
            let headers = String::from_utf8(request.clone()).unwrap();
            let size = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap();
            let mut body = vec![0; size];
            stream.read_exact(&mut body).unwrap();
            request.extend(body);
            let _ = send.send(String::from_utf8(request).unwrap());
            for part in response.as_bytes().chunks(7) {
                if stream.write_all(part).is_err() {
                    break;
                }
            }
            let _ = stream.flush();
            std::thread::sleep(delay);
        });
        let base = format!("http://{address}/v1");
        let suffix = match protocol {
            Protocol::ChatCompletions => "chat/completions",
            Protocol::Responses => "responses",
            Protocol::AnthropicMessages => "messages",
            Protocol::Gemini => "",
        };
        let client = Client {
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .build()
                .unwrap(),
            protocol,
            url: Url::parse(&format!("{base}/{suffix}")).unwrap(),
            base,
            limits,
            transport: GenerationTransport::Stream,
            channel_id: None,
        };
        (client, receive, handle)
    }
    fn input() -> Request {
        Request {
            model: "fixture".into(),
            messages: vec![Message::User {
                text: "synthetic".into(),
            }],
            tools: vec![],
            max_output_tokens: 64,
            chat_token_limit: None,
            temperature: None,
            output_format: None,
        }
    }
    fn response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }
    fn chat_text() -> &'static str {
        "data: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"中\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
    }
    #[tokio::test]
    async fn real_http_transport_preserves_headers_payload_and_result() {
        let (client, wire, peer) = spawn_peer(
            Protocol::ChatCompletions,
            response(chat_text()),
            Duration::ZERO,
            Limits::default(),
        );
        let mut events = Vec::new();
        let result = client
            .generate(
                &input(),
                &ApiKey::new("fixture-secret".into()).unwrap(),
                |e| {
                    events.push(e);
                    Ok(())
                },
                || false,
            )
            .await
            .unwrap();
        assert_eq!(result.text, "中");
        assert_eq!(result.finish, Finish::Stop);
        assert_eq!(events.len(), 1);
        let request = wire.recv().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions "));
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer fixture-secret")
        );
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["stream"], true);
        assert!(!body.to_string().contains("fixture-secret"));
        peer.join().unwrap();
    }
    #[tokio::test]
    async fn anthropic_transport_sends_native_auth_and_version() {
        let body = "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\"}}\n\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n";
        let (client, wire, peer) = spawn_peer(
            Protocol::AnthropicMessages,
            response(body),
            Duration::ZERO,
            Limits::default(),
        );
        assert_eq!(
            client
                .generate(
                    &input(),
                    &ApiKey::new("fixture".into()).unwrap(),
                    |_| Ok(()),
                    || false
                )
                .await
                .unwrap()
                .finish,
            Finish::Stop
        );
        let request = wire.recv().unwrap().to_lowercase();
        assert!(request.contains("x-api-key: fixture"));
        assert!(request.contains("anthropic-version: 2023-06-01"));
        assert!(!request.contains("authorization:"));
        peer.join().unwrap();
    }
    #[tokio::test]
    async fn status_failures_are_redacted_and_never_follow_redirects() {
        for (status, expected) in [
            (401, Error::Authentication),
            (403, Error::Authentication),
            (429, Error::RateLimited),
            (400, Error::RequestRejected),
            (404, Error::RequestRejected),
            (500, Error::ApiError),
            (307, Error::ApiError),
        ] {
            let response = format!(
                "HTTP/1.1 {status} Test\r\nLocation: http://127.0.0.1:1/leak\r\nContent-Length: 14\r\nConnection: close\r\n\r\nprivate-secret"
            );
            let (client, _, peer) = spawn_peer(
                Protocol::Responses,
                response,
                Duration::ZERO,
                Limits::default(),
            );
            let result = client
                .generate(
                    &input(),
                    &ApiKey::new("fixture".into()).unwrap(),
                    |_| Ok(()),
                    || false,
                )
                .await;
            assert_eq!(result.err(), Some(expected));
            assert!(!expected.to_string().contains("private"));
            peer.join().unwrap();
        }
    }
    #[tokio::test]
    async fn real_http_eof_limit_and_non_sse_are_rejected() {
        for (body, limit, expected) in [
            (
                "data: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"},\"finish_reason\":null}]}\n\n",
                4096,
                Error::Incomplete,
            ),
            (chat_text(), 20, Error::ResponseTooLarge),
        ] {
            let limits = Limits {
                max_response_bytes: limit,
                max_event_bytes: limit,
                ..Default::default()
            };
            let (client, _, peer) = spawn_peer(
                Protocol::ChatCompletions,
                response(body),
                Duration::ZERO,
                limits,
            );
            assert_eq!(
                client
                    .generate(
                        &input(),
                        &ApiKey::new("fixture".into()).unwrap(),
                        |_| Ok(()),
                        || false
                    )
                    .await
                    .err(),
                Some(expected)
            );
            peer.join().unwrap();
        }
        let (client, _, peer) = spawn_peer(
            Protocol::ChatCompletions,
            response("{}").replace("text/event-stream", "application/json"),
            Duration::ZERO,
            Limits::default(),
        );
        assert_eq!(
            client
                .generate(
                    &input(),
                    &ApiKey::new("fixture".into()).unwrap(),
                    |_| Ok(()),
                    || false
                )
                .await
                .err(),
            Some(Error::InvalidResponse)
        );
        peer.join().unwrap();
    }
    #[tokio::test]
    async fn real_wait_is_cancelable_and_callback_failure_stops_stream() {
        let response_header =
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 10000\r\n\r\n"
                .to_owned();
        let (client, _, peer) = spawn_peer(
            Protocol::ChatCompletions,
            response_header,
            Duration::from_millis(350),
            Limits::default(),
        );
        let started = Instant::now();
        let result = client
            .generate(
                &input(),
                &ApiKey::new("fixture".into()).unwrap(),
                |_| Ok(()),
                || started.elapsed() > Duration::from_millis(40),
            )
            .await;
        assert_eq!(result.err(), Some(Error::Canceled));
        assert!(started.elapsed() < Duration::from_millis(300));
        peer.join().unwrap();
        let (client, _, peer) = spawn_peer(
            Protocol::ChatCompletions,
            response(chat_text()),
            Duration::ZERO,
            Limits::default(),
        );
        assert_eq!(
            client
                .generate(
                    &input(),
                    &ApiKey::new("fixture".into()).unwrap(),
                    |_| Err(Error::Canceled),
                    || false
                )
                .await
                .err(),
            Some(Error::Canceled)
        );
        peer.join().unwrap();
    }
    #[test]
    fn endpoints_are_explicit_https_roots() {
        for p in [
            Protocol::ChatCompletions,
            Protocol::Responses,
            Protocol::AnthropicMessages,
        ] {
            let (_, url) = endpoint(p, "https://example.test/custom/v1/").unwrap();
            assert!(url.path().starts_with("/custom/v1/"));
        }
        assert_eq!(
            endpoint(Protocol::ChatCompletions, "http://127.0.0.1:11436/v1")
                .unwrap()
                .1
                .as_str(),
            "http://127.0.0.1:11436/v1/chat/completions"
        );
        assert!(endpoint(Protocol::ChatCompletions, "http://127.0.0.1:11436/v1?key=x").is_err());
        for bad in [
            "http://example.test/v1",
            "https://user:secret@example.test/v1",
            "https://example.test/v1?key=secret",
            "https://example.test/#x",
            "https://example.test/\n",
            "https:\\example.test/v1",
        ] {
            assert!(endpoint(Protocol::Responses, bad).is_err());
        }
        assert_eq!(
            endpoint(Protocol::Responses, "https://example.test")
                .unwrap()
                .1
                .as_str(),
            "https://example.test/responses"
        );
    }

    #[tokio::test]
    async fn real_wait_enforces_first_event_and_idle_timeouts() {
        let limits = Limits {
            first_event_timeout: Duration::from_millis(40),
            idle_timeout: Duration::from_millis(40),
            ..Default::default()
        };
        for (body, expected) in [
            ("", Error::FirstEventTimeout),
            (
                "data: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"},\"finish_reason\":null}]}\n\n",
                Error::IdleTimeout,
            ),
        ] {
            let wire = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 10000\r\n\r\n{body}"
            );
            let (client, _, peer) = spawn_peer(
                Protocol::ChatCompletions,
                wire,
                Duration::from_millis(250),
                limits,
            );
            assert_eq!(
                client
                    .generate(
                        &input(),
                        &ApiKey::new("fixture".into()).unwrap(),
                        |_| Ok(()),
                        || false
                    )
                    .await
                    .err(),
                Some(expected)
            );
            peer.join().unwrap();
        }
    }

    #[tokio::test]
    async fn responses_http_transport_handles_terminal_items() {
        let body=[serde_json::json!({"type":"response.created","response":{"id":"r"}}),serde_json::json!({"type":"response.output_item.added","output_index":0,"item":{"id":"m","type":"message","content":[]}}),serde_json::json!({"type":"response.output_text.delta","output_index":0,"item_id":"m","delta":"ok"}),serde_json::json!({"type":"response.output_item.done","output_index":0,"item":{"id":"m","type":"message","content":[{"type":"output_text","text":"ok"}]}}),serde_json::json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[{"id":"m","type":"message","content":[{"type":"output_text","text":"ok"}]}]}})].into_iter().map(|v|format!("data: {v}\n\n")).collect::<String>();
        let (client, wire, peer) = spawn_peer(
            Protocol::Responses,
            response(&body),
            Duration::ZERO,
            Limits::default(),
        );
        let result = client
            .generate(
                &input(),
                &ApiKey::new("fixture".into()).unwrap(),
                |_| Ok(()),
                || false,
            )
            .await
            .unwrap();
        assert_eq!(result.text, "ok");
        assert_eq!(result.finish, Finish::Stop);
        assert!(wire.recv().unwrap().starts_with("POST /v1/responses "));
        peer.join().unwrap();
    }
    #[test]
    fn keys_are_sensitive_and_header_injection_is_rejected() {
        let key = ApiKey::new("test-only-secret".into()).unwrap();
        assert!(key.0.is_sensitive());
        assert!(!format!("{:?}", key.0).contains("test-only-secret"));
        for key in ["", "secret\r\nX-Evil: yes", " white ", "中"] {
            assert!(ApiKey::new(key.into()).is_err());
        }
    }
    #[test]
    fn timeout_categories_and_cancellation() {
        let limits = Limits::default();
        let now = Instant::now();
        assert_eq!(check(limits, now, None, &mut || true), Err(Error::Canceled));
        assert_eq!(
            check(limits, now - Duration::from_secs(300), None, &mut || false),
            Err(Error::TotalTimeout)
        );
        assert_eq!(
            check(limits, now - Duration::from_secs(100), None, &mut || false),
            Err(Error::FirstEventTimeout)
        );
        assert_eq!(
            check(
                limits,
                now - Duration::from_secs(40),
                Some(now - Duration::from_secs(35)),
                &mut || false
            ),
            Err(Error::IdleTimeout)
        );
    }
}

//! OpenAI-compatible speech-to-text transport. Audio is sent only from this
//! module, never from the React layer, and the API key never leaves the OS vault.
use crate::{AppError, Result, WorkspaceIdentity, inference_credentials as vault};
use pkms_core::{AsrSettings, VoiceSegment};
use reqwest::blocking::multipart::{Form, Part};
use std::{
    io::Read,
    time::{Duration, Instant},
};

const ASR_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

pub struct AsrOutcome {
    pub text: String,
    pub segments: Vec<VoiceSegment>,
}

pub(crate) struct AsrJob {
    pub settings: AsrSettings,
    pub secret: vault::Secret,
    pub audio: Vec<u8>,
    pub media_type: String,
    pub row_version: u32,
}

/// Opaque prepared transcription. The native adapter owns the network call while
/// the global service lock is released.
pub struct VoiceTranscription {
    pub(crate) job: AsrJob,
    pub(crate) capture_id: String,
}

impl VoiceTranscription {
    pub fn capture_id(&self) -> &str {
        &self.capture_id
    }

    pub fn row_version(&self) -> u32 {
        self.job.row_version
    }

    pub fn run(&self) -> Result<AsrOutcome> {
        transcribe(&self.job, None)
    }
}

pub struct AsrProbe {
    pub(crate) settings: AsrSettings,
    pub(crate) secret: vault::Secret,
}

impl AsrProbe {
    pub fn run(&self) -> Result<u32> {
        test(&self.settings, &self.secret)
    }
}

pub fn credential_binding(
    identity: &WorkspaceIdentity,
    settings: &AsrSettings,
) -> Result<vault::CredentialBinding> {
    vault::CredentialBinding::new_asr(
        &identity.local_instance_id,
        &identity.workspace_id,
        &settings.endpoint,
        &settings.model,
    )
    .map_err(crate::inference_remote::credential_error)
}

pub fn credential_configured(identity: &WorkspaceIdentity, settings: &AsrSettings) -> Result<bool> {
    if settings.endpoint.is_empty() || settings.model.is_empty() {
        return Ok(false);
    }
    Ok(vault::load(&credential_binding(identity, settings)?)
        .map_err(crate::inference_remote::credential_error)?
        .is_some())
}

pub fn load_secret(identity: &WorkspaceIdentity, settings: &AsrSettings) -> Result<vault::Secret> {
    vault::load(&credential_binding(identity, settings)?)
        .map_err(crate::inference_remote::credential_error)?
        .ok_or(AppError::InvalidInput("asr_credential_missing"))
}

pub fn store_secret(
    identity: &WorkspaceIdentity,
    settings: &AsrSettings,
    secret: &str,
) -> Result<()> {
    let binding = credential_binding(identity, settings)?;
    let secret =
        vault::Secret::new(secret.to_owned()).map_err(crate::inference_remote::credential_error)?;
    vault::store(&binding, &secret).map_err(crate::inference_remote::credential_error)
}

pub fn clear_secret(identity: &WorkspaceIdentity, settings: &AsrSettings) -> Result<()> {
    let binding = credential_binding(identity, settings)?;
    vault::delete(&binding).map_err(crate::inference_remote::credential_error)
}

/// Loopback ASR never sends audio off the machine and is exempt from the
/// remote-data acknowledgement, matching generation and embedding behavior.
pub fn is_remote(settings: &AsrSettings) -> bool {
    let Ok(url) = reqwest::Url::parse(&settings.endpoint) else {
        return true;
    };
    !url.host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|address| address.is_loopback())
}

pub fn transcribe(job: &AsrJob, language_override: Option<&str>) -> Result<AsrOutcome> {
    let language = language_override
        .filter(|value| !value.is_empty())
        .unwrap_or(job.settings.language.as_str());
    post(
        &job.settings.endpoint,
        &job.settings.model,
        language,
        &job.secret,
        &job.audio,
        &job.media_type,
    )
}

pub fn test(settings: &AsrSettings, secret: &vault::Secret) -> Result<u32> {
    let started = Instant::now();
    post(
        &settings.endpoint,
        &settings.model,
        &settings.language,
        secret,
        &silence_wav(),
        "audio/wav",
    )?;
    Ok(u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX))
}

fn post(
    endpoint: &str,
    model: &str,
    language: &str,
    secret: &vault::Secret,
    bytes: &[u8],
    media_type: &str,
) -> Result<AsrOutcome> {
    if bytes.is_empty() || bytes.len() > 25 * 1024 * 1024 {
        return Err(AppError::InvalidInput("asr_audio_too_large"));
    }
    let url = transcription_url(endpoint)?;
    let part = Part::bytes(bytes.to_vec())
        .file_name(format!("voice.{}", extension(media_type)))
        .mime_str(media_type)
        .map_err(|_| AppError::InvalidInput("asr_audio_invalid"))?;
    let mut form = Form::new()
        .text("model", model.to_owned())
        .text("response_format", "verbose_json")
        .part("file", part);
    if !language.trim().is_empty() {
        form = form.text("language", language.trim().to_owned());
    }
    let response = client()?
        .post(url)
        .bearer_auth(secret.expose_secret())
        .multipart(form)
        .send()
        .map_err(request_error)?;
    let status = response.status();
    let mut body = String::new();
    response
        .take(MAX_RESPONSE_BYTES)
        .read_to_string(&mut body)
        .map_err(|_| AppError::InvalidInput("asr_invalid_response"))?;
    if !status.is_success() {
        return Err(status_error(status.as_u16()));
    }
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| AppError::InvalidInput("asr_invalid_response"))?;
    let text = value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(AppError::InvalidInput("asr_invalid_response"))?;
    let mut segments = value
        .get("segments")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|segment| {
                    let start = segment.get("start")?.as_f64()?;
                    let end = segment.get("end")?.as_f64()?;
                    let text = segment.get("text")?.as_str()?.trim();
                    if text.is_empty() || !start.is_finite() || !end.is_finite() || end < start {
                        return None;
                    }
                    let speaker = segment
                        .get("speaker")
                        .or_else(|| segment.get("speaker_label"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned);
                    Some(VoiceSegment {
                        start_ms: (start.max(0.0) * 1000.0).round() as u32,
                        end_ms: (end.max(0.0) * 1000.0).round() as u32,
                        text: text.to_owned(),
                        speaker,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    segments.sort_by_key(|segment| segment.start_ms);
    Ok(AsrOutcome { text, segments })
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(ASR_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| AppError::InvalidInput("asr_client_unavailable"))
}

fn transcription_url(endpoint: &str) -> Result<String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        return Err(AppError::InvalidInput("asr_not_configured"));
    }
    let url = if endpoint.ends_with("/audio/transcriptions") {
        endpoint.to_owned()
    } else {
        format!("{endpoint}/audio/transcriptions")
    };
    let parsed =
        reqwest::Url::parse(&url).map_err(|_| AppError::InvalidInput("asr_endpoint_invalid"))?;
    if !(parsed.scheme() == "https"
        || (parsed.scheme() == "http"
            && parsed
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|address| address.is_loopback())))
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AppError::InvalidInput("asr_endpoint_invalid"));
    }
    Ok(url)
}

fn request_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        AppError::InvalidInput("asr_timeout")
    } else {
        AppError::InvalidInput("asr_network_error")
    }
}

fn status_error(status: u16) -> AppError {
    AppError::InvalidInput(match status {
        401 | 403 => "asr_auth_failed",
        408 => "asr_timeout",
        413 => "asr_audio_too_large",
        429 => "asr_rate_limited",
        500..=599 => "asr_service_unavailable",
        _ => "asr_request_rejected",
    })
}

fn extension(media_type: &str) -> &'static str {
    match media_type {
        "audio/webm" => "webm",
        "audio/ogg" => "ogg",
        "audio/wav" => "wav",
        "audio/mp4" => "m4a",
        "audio/mpeg" => "mp3",
        _ => "bin",
    }
}

/// 0.5 s of 16 kHz mono PCM silence; enough to verify endpoint, auth and JSON.
fn silence_wav() -> Vec<u8> {
    let samples: u32 = 8_000;
    let data_len = samples * 2;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&16_000u32.to_le_bytes());
    bytes.extend_from_slice(&32_000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.resize(44 + data_len as usize, 0);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    #[test]
    fn silence_probe_is_a_valid_wav() {
        let wav = silence_wav();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 16_044);
    }

    #[test]
    fn endpoint_appends_the_transcription_path_once() {
        assert_eq!(
            transcription_url("https://example.test/v1").unwrap(),
            "https://example.test/v1/audio/transcriptions"
        );
        assert_eq!(
            transcription_url("https://example.test/v1/audio/transcriptions/").unwrap(),
            "https://example.test/v1/audio/transcriptions"
        );
        assert!(transcription_url("http://example.test/v1").is_err());
    }

    #[test]
    fn multipart_request_reaches_a_local_peer_and_returns_text() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let captured = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&captured);
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = socket.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            let header_end = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let head = String::from_utf8_lossy(&request[..header_end]).to_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while request.len() < header_end + length {
                let read = socket.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            *sink.lock().unwrap() = String::from_utf8_lossy(&request).into_owned();
            let body = r#"{"text":"本地合成转写"}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let secret = vault::Secret::new("synthetic-key".into()).unwrap();
        let outcome = post(
            &endpoint,
            "whisper-1",
            "zh",
            &secret,
            b"\x1a\x45\xdf\xa3synthetic-audio",
            "audio/webm",
        )
        .unwrap();
        assert_eq!(outcome.text, "本地合成转写");
        assert!(outcome.segments.is_empty());
        let request = captured.lock().unwrap();
        let lower = request.to_lowercase();
        assert!(
            lower.contains("authorization: bearer synthetic-key"),
            "captured request headers: {request}"
        );
        assert!(lower.contains("name=\"model\""));
        assert!(lower.contains("name=\"file\""));
        assert!(lower.contains("audio/webm"));
    }
}

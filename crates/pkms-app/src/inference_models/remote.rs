//! Read-only remote catalogs. Listing does not verify generation or replace saved models.
use super::run_model_request;
use crate::{
    AppError, Result, WorkspaceIdentity, inference_credentials as vault, inference_remote,
};
use pkms_core::{
    DiscoveredGenerationModel, GenerationChannel, GenerationChannelPreset, RemoteGenerationModels,
};
use reqwest::{Url, header};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 64;
const MAX_MODELS: usize = 4096;
const MAX_PAGE_BYTES: usize = 1024 * 1024;
const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;

struct RemoteService {
    client: reqwest::Client,
    endpoint: Url,
    provider: String,
    cloudflare: bool,
    headers: header::HeaderMap,
    bytes: usize,
}

fn invalid() -> AppError {
    AppError::InvalidInput("model_response_invalid")
}
fn limit() -> AppError {
    AppError::InvalidInput("model_response_limit")
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| identifier(value))
        .ok_or_else(invalid)
}
fn optional_string(value: &Value, key: &str) -> Result<Option<String>> {
    value
        .get(key)
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_str()
                .filter(|value| {
                    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
                })
                .map(str::to_owned)
                .ok_or_else(invalid)
        })
        .transpose()
}
fn optional_tokens(value: &Value, key: &str) -> Result<Option<u32>> {
    value
        .get(key)
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(invalid)
        })
        .transpose()
}
fn page_items<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .filter(|items| items.len() <= MAX_MODELS)
        .map(Vec::as_slice)
        .ok_or_else(invalid)
}

impl RemoteService {
    fn new(channel: &GenerationChannel, secret: &vault::Secret) -> Result<Self> {
        let endpoint = Url::parse(channel.spec.endpoint.trim_end_matches('/'))
            .map_err(|_| AppError::InvalidInput("invalid_model_endpoint"))?;
        let loopback_http = endpoint.scheme() == "http"
            && endpoint
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|address| address.is_loopback());
        let client = reqwest::Client::builder()
            .https_only(!loopback_http)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| AppError::InvalidInput("model_unavailable"))?;
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );
        let (name, raw) = match channel.spec.provider.as_str() {
            "gemini" => ("x-goog-api-key", secret.expose_secret().to_owned()),
            "anthropic" => {
                headers.insert(
                    "anthropic-version",
                    header::HeaderValue::from_static("2023-06-01"),
                );
                ("x-api-key", secret.expose_secret().to_owned())
            }
            "openai_chat" | "openai_responses" => (
                "authorization",
                format!("Bearer {}", secret.expose_secret()),
            ),
            _ => return Err(AppError::InvalidInput("model_profile_unsupported")),
        };
        let mut value = header::HeaderValue::from_str(&raw)
            .map_err(|_| AppError::InvalidInput("model_invalid_key"))?;
        value.set_sensitive(true);
        headers.insert(name, value);
        let cloudflare = channel.spec.preset == Some(GenerationChannelPreset::CloudflareWorkersAi)
            || (endpoint.host_str() == Some("api.cloudflare.com")
                && endpoint.path().ends_with("/ai/v1"));
        Ok(Self {
            client,
            endpoint,
            provider: channel.spec.provider.clone(),
            cloudflare,
            headers,
            bytes: 0,
        })
    }

    fn models_url(&self) -> Result<Url> {
        let mut url = self.endpoint.clone();
        if self.cloudflare {
            let prefix = url
                .path()
                .trim_end_matches('/')
                .strip_suffix("/ai/v1")
                .ok_or(AppError::InvalidInput("invalid_model_endpoint"))?
                .to_owned();
            url.set_path(&format!("{prefix}/ai/models/search"));
        } else {
            url.path_segments_mut()
                .map_err(|_| AppError::InvalidInput("invalid_model_endpoint"))?
                .pop_if_empty()
                .push("models");
        }
        Ok(url)
    }

    async fn get(&mut self, url: Url) -> Result<Value> {
        let mut response = self
            .client
            .get(url)
            .headers(self.headers.clone())
            .send()
            .await
            .map_err(|_| AppError::InvalidInput("model_unavailable"))?;
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(AppError::InvalidInput("model_authentication_failed")),
            429 => return Err(AppError::InvalidInput("model_rate_limited")),
            404 | 405 => return Err(AppError::InvalidInput("model_discovery_unsupported")),
            400..=499 => return Err(AppError::InvalidInput("model_request_rejected")),
            _ => return Err(AppError::InvalidInput("model_api_error")),
        }
        if response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_none_or(|value| !value.trim().eq_ignore_ascii_case("application/json"))
        {
            return Err(invalid());
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_PAGE_BYTES as u64)
        {
            return Err(limit());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AppError::InvalidInput("model_unavailable"))?
        {
            self.bytes = self.bytes.checked_add(chunk.len()).ok_or_else(limit)?;
            if body.len() + chunk.len() > MAX_PAGE_BYTES || self.bytes > MAX_CATALOG_BYTES {
                return Err(limit());
            }
            body.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&body).map_err(|_| invalid())?;
        if !value.is_object() {
            return Err(invalid());
        }
        if value.get("error").is_some_and(|value| !value.is_null())
            || value.get("success").is_some_and(|value| value != true)
            || value
                .get("errors")
                .is_some_and(|value| value.as_array().is_none_or(|errors| !errors.is_empty()))
        {
            return Err(AppError::InvalidInput("model_api_error"));
        }
        Ok(value)
    }

    async fn discover(&mut self) -> Result<Vec<DiscoveredGenerationModel>> {
        let base = self.models_url()?;
        let mut catalog = BTreeMap::<String, DiscoveredGenerationModel>::new();
        let mut cursors = HashSet::new();
        let mut seen_names = HashSet::new();
        let mut cursor = None::<String>;
        for page in 1..=MAX_PAGES {
            let mut url = base.clone();
            if self.cloudflare {
                url.query_pairs_mut()
                    .append_pair("page", &page.to_string())
                    .append_pair("per_page", &PAGE_SIZE.to_string());
            } else if self.provider == "gemini" {
                url.query_pairs_mut()
                    .append_pair("pageSize", &PAGE_SIZE.to_string());
                if let Some(cursor) = &cursor {
                    url.query_pairs_mut().append_pair("pageToken", cursor);
                }
            } else if self.provider == "anthropic" {
                url.query_pairs_mut()
                    .append_pair("limit", &PAGE_SIZE.to_string());
                if let Some(cursor) = &cursor {
                    url.query_pairs_mut().append_pair("after_id", cursor);
                }
            }
            let body = self.get(url).await?;
            let key = if self.cloudflare {
                "result"
            } else if self.provider == "gemini" {
                "models"
            } else {
                "data"
            };
            if self.cloudflare && body["success"] != true {
                return Err(invalid());
            }
            let items = page_items(&body, key)?;
            let mut new_names = 0usize;
            for item in items {
                if !item.is_object() {
                    return Err(invalid());
                }
                let name = required_string(
                    item,
                    if self.cloudflare || self.provider == "gemini" {
                        "name"
                    } else {
                        "id"
                    },
                )?;
                if seen_names.insert(name.to_owned()) {
                    new_names += 1;
                }
                if seen_names.len() > MAX_MODELS {
                    return Err(limit());
                }
                let model = if self.cloudflare {
                    let task = item
                        .get("task")
                        .and_then(|task| {
                            task.as_str()
                                .or_else(|| task.get("name").and_then(Value::as_str))
                        })
                        .ok_or(AppError::InvalidInput("model_discovery_unsupported"))?;
                    if !matches!(
                        task.to_ascii_lowercase().as_str(),
                        "text generation" | "text-generation" | "text_generation"
                    ) {
                        continue;
                    }
                    DiscoveredGenerationModel {
                        model: name.to_owned(),
                        display_name: optional_string(item, "display_name")?,
                        context_tokens: optional_tokens(item, "context_length")?,
                    }
                } else if self.provider == "gemini" {
                    let methods = item
                        .get("supportedGenerationMethods")
                        .map(|value| value.as_array().ok_or_else(invalid))
                        .transpose()?;
                    let mut generation = false;
                    if let Some(methods) = methods {
                        for method in methods {
                            let method = method.as_str().ok_or_else(invalid)?;
                            generation |=
                                matches!(method, "generateContent" | "streamGenerateContent");
                        }
                    }
                    if !generation {
                        continue;
                    }
                    let model = name.strip_prefix("models/").ok_or_else(invalid)?;
                    if model.is_empty()
                        || matches!(model, "." | "..")
                        || !model
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
                    {
                        return Err(invalid());
                    }
                    DiscoveredGenerationModel {
                        model: model.to_owned(),
                        display_name: optional_string(item, "displayName")?,
                        context_tokens: optional_tokens(item, "inputTokenLimit")?,
                    }
                } else {
                    DiscoveredGenerationModel {
                        model: name.to_owned(),
                        display_name: optional_string(item, "display_name")?,
                        context_tokens: optional_tokens(item, "context_length")?
                            .or(optional_tokens(item, "max_input_tokens")?),
                    }
                };
                if let Some(old) = catalog.get(&model.model) {
                    if old.display_name != model.display_name
                        || old.context_tokens != model.context_tokens
                    {
                        return Err(invalid());
                    }
                } else {
                    catalog.insert(model.model.clone(), model);
                }
            }
            // An ignored cursor/page must not produce a successful but incomplete catalog.
            if page > 1 && !items.is_empty() && new_names == 0 {
                return Err(invalid());
            }
            let more = if self.cloudflare {
                if let Some(info) = body.get("result_info") {
                    if !info.is_object() {
                        return Err(invalid());
                    }
                    if info
                        .get("page")
                        .is_some_and(|value| value.as_u64() != Some(page as u64))
                    {
                        return Err(invalid());
                    }
                    if let Some(total) = info.get("total_pages") {
                        let total = total.as_u64().ok_or_else(invalid)?;
                        if total < page as u64 && !(page == 1 && total == 0 && items.is_empty()) {
                            return Err(invalid());
                        }
                        (page as u64) < total
                    } else {
                        !items.is_empty()
                    }
                } else {
                    // Cloudflare does not promise page-size defaults/caps or result_info.
                    // A short page is not proof of completeness: continue to an empty page.
                    !items.is_empty()
                }
            } else if self.provider == "gemini" {
                cursor = body
                    .get("nextPageToken")
                    .map(|value| {
                        value
                            .as_str()
                            .filter(|value| {
                                value.len() <= 4096 && !value.chars().any(char::is_control)
                            })
                            .map(str::to_owned)
                            .ok_or_else(invalid)
                    })
                    .transpose()?
                    .filter(|value| !value.is_empty());
                cursor.is_some()
            } else if self.provider == "anthropic" {
                let more = body["has_more"].as_bool().ok_or_else(invalid)?;
                if more {
                    let last = required_string(&body, "last_id")?;
                    if items.last().and_then(|item| item["id"].as_str()) != Some(last) {
                        return Err(invalid());
                    }
                    cursor = Some(last.to_owned());
                }
                more
            } else {
                // The OpenAI /models contract returns the complete list, without a cursor.
                if body.get("has_more").is_some_and(|value| value != false)
                    || body.get("next_page").is_some_and(|value| !value.is_null())
                {
                    return Err(AppError::InvalidInput("model_discovery_unsupported"));
                }
                false
            };
            if !more {
                return Ok(catalog.into_values().collect());
            }
            if items.is_empty() {
                return Err(invalid());
            }
            if let Some(cursor) = &cursor
                && !cursors.insert(cursor.clone())
            {
                return Err(invalid());
            }
        }
        Err(limit())
    }
}

pub fn list_remote_generation_models(
    identity: &WorkspaceIdentity,
    channel: &GenerationChannel,
    shutdown: &AtomicBool,
) -> Result<RemoteGenerationModels> {
    if shutdown.load(Ordering::Acquire) {
        return Err(AppError::InvalidInput("worker_interrupted"));
    }
    let binding = inference_remote::generation_channel_credential_binding(identity, channel)?;
    let secret = match vault::load(&binding).map_err(inference_remote::credential_error)? {
        Some(secret) => secret,
        None if channel.spec.endpoint.starts_with("http://127.0.0.1")
            || channel.spec.endpoint.starts_with("http://[::1]") =>
        {
            vault::Secret::new("local".into()).map_err(inference_remote::credential_error)?
        }
        None => return Err(AppError::InvalidInput("model_key_missing")),
    };
    let mut service = RemoteService::new(channel, &secret)?;
    let models = run_model_request(shutdown, async {
        tokio::time::timeout(Duration::from_secs(30), service.discover())
            .await
            .map_err(|_| AppError::InvalidInput("model_total_timeout"))?
    })?;
    Ok(RemoteGenerationModels {
        channel_id: channel.id.clone(),
        row_version: channel.row_version,
        provider: channel.spec.provider.clone(),
        endpoint: channel.spec.endpoint.clone(),
        models,
    })
}

#[cfg(test)]
mod tests;

//! Admission for the locally verified text-chat path, not a model quality rating.
use crate::{AppError, Result};
use pkms_core::GenerationProfile;
use serde_json::Value;
use sha2::{Digest, Sha256};

const MODEL: &str = "qwen3:0.6b";
// Registry manifest / api/tags digest, deliberately not the GGUF weight digest.
const MANIFEST: &str = "7df6b6e09427a769808717c0a93cadc4ae99ed4eb8bf5ca557c90846becea435";
const TEMPLATE_SHA256: &str = "ae370d884f108d16e7cc8fd5259ebc5773a0afa6e078b11f4ed7e39a27e0dfc4";
const MODEL_4B: &str = "qwen3.5:4b";
const MANIFEST_4B: &str = "2a654d98e6fba55d452b7043684e9b57a947e393bbffa62485a7aac05ee4eefd";
const RENDERER_RUNTIME: &str = "0.35.1";
pub(crate) const MODEL_CONTEXT_RANGE: (u32, u32) = (2048, 8192);

/// A locally installed identity candidate, still requiring runtime/show checks.
/// This is protocol admission only, never a model-quality recommendation.
pub(crate) fn candidate_profile(
    endpoint: &str,
    model: &str,
    digest: &str,
) -> Option<GenerationProfile> {
    let profile = GenerationProfile {
        channel_id: None,
        capabilities: None,
        chat_token_limit: None,
        provider: "ollama".into(),
        endpoint: endpoint.into(),
        model: model.into(),
        model_digest: digest.strip_prefix("sha256:").unwrap_or(digest).into(),
        config_revision: 1,
        temperature: 0.0,
        context_tokens: MODEL_CONTEXT_RANGE.1,
    };
    validate_profile(&profile).ok().map(|()| profile)
}

fn unsupported() -> AppError {
    AppError::InvalidInput("model_profile_unsupported")
}

/// Bound the admitted model and context before applying the chat context estimate.
/// This template was checked with Ollama 0.35.1 and real local inference. The
/// caller must use the text-only system/user path, think=false, without tools,
/// images, baked-in conversation or arbitrary extra messages. Its byte-level
/// tokenizer and fixed framing make the existing byte + wrapper bound conservative.
/// The qwen3.5 renderer additionally requires the exact checked runtime version.
pub(crate) fn validate_profile(profile: &GenerationProfile) -> Result<()> {
    let digest = profile
        .model_digest
        .strip_prefix("sha256:")
        .unwrap_or(&profile.model_digest);
    if profile.provider != "ollama"
        || !matches!(
            (profile.model.as_str(), digest),
            (MODEL, MANIFEST) | (MODEL_4B, MANIFEST_4B)
        )
        || !(MODEL_CONTEXT_RANGE.0..=MODEL_CONTEXT_RANGE.1).contains(&profile.context_tokens)
        || profile.config_revision == 0
        || !profile.temperature.is_finite()
        || !(0.0..=2.0).contains(&profile.temperature)
    {
        return Err(unsupported());
    }
    Ok(())
}

/// Validate real /api/show output in addition to the separate /api/tags identity
/// check. A matching tag alone cannot justify a prompt/template token bound.
pub(crate) fn validate_show(
    profile: &GenerationProfile,
    show: &Value,
    runtime_version: &str,
) -> Result<()> {
    validate_profile(profile)?;
    let is_4b = profile.model == MODEL_4B;
    let template = show
        .get("template")
        .and_then(Value::as_str)
        .ok_or_else(unsupported)?;
    let template_hash = format!("{:x}", Sha256::digest(template.as_bytes()));
    let system_empty = show.get("system").is_none_or(|v| v.as_str() == Some(""));
    let messages_empty = show
        .get("messages")
        .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty));
    // The 4B manifest selects Ollama's built-in Qwen35Renderer, not this displayed
    // fallback template. Pin the inspected runtime as well as the model manifest.
    let template_matches = if is_4b {
        runtime_version == RENDERER_RUNTIME
            && template == "{{ .Prompt }}"
            && show.get("requires").and_then(Value::as_str) == Some("0.17.1")
    } else {
        template_hash == TEMPLATE_SHA256
    };
    let family = if is_4b { "qwen35" } else { "qwen3" };
    if !template_matches
        || !system_empty
        || !messages_empty
        || show.pointer("/details/format").and_then(Value::as_str) != Some("gguf")
        || show.pointer("/details/family").and_then(Value::as_str) != Some(family)
        || show
            .pointer("/details/quantization_level")
            .and_then(Value::as_str)
            != Some("Q4_K_M")
        || show
            .pointer("/model_info/general.architecture")
            .and_then(Value::as_str)
            != Some(family)
        || show
            .pointer("/model_info/tokenizer.ggml.model")
            .and_then(Value::as_str)
            != Some("gpt2")
        || show
            .pointer(&format!("/model_info/{family}.context_length"))
            .and_then(Value::as_u64)
            != Some(if is_4b { 262144 } else { 40960 })
        || !show
            .get("capabilities")
            .and_then(Value::as_array)
            .is_some_and(|caps| caps.iter().any(|v| v.as_str() == Some("completion")))
    {
        return Err(unsupported());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Official template layer, also byte-identical to the real /api/show result.
    const VERIFIED_TEMPLATE: &str = r###"
{{- $lastUserIdx := -1 -}}
{{- range $idx, $msg := .Messages -}}
{{- if eq $msg.Role "user" }}{{ $lastUserIdx = $idx }}{{ end -}}
{{- end }}
{{- if or .System .Tools }}<|im_start|>system
{{ if .System }}
{{ .System }}
{{- end }}
{{- if .Tools }}

# Tools

You may call one or more functions to assist with the user query.

You are provided with function signatures within <tools></tools> XML tags:
<tools>
{{- range .Tools }}
{"type": "function", "function": {{ .Function }}}
{{- end }}
</tools>

For each function call, return a json object with function name and arguments within <tool_call></tool_call> XML tags:
<tool_call>
{"name": <function-name>, "arguments": <args-json-object>}
</tool_call>
{{- end -}}
<|im_end|>
{{ end }}
{{- range $i, $_ := .Messages }}
{{- $last := eq (len (slice $.Messages $i)) 1 -}}
{{- if eq .Role "user" }}<|im_start|>user
{{ .Content }}
{{- if and $.IsThinkSet (eq $i $lastUserIdx) }}
   {{- if $.Think -}}
      {{- " "}}/think
   {{- else -}}
      {{- " "}}/no_think
   {{- end -}}
{{- end }}<|im_end|>
{{ else if eq .Role "assistant" }}<|im_start|>assistant
{{ if (and $.IsThinkSet (and .Thinking (or $last (gt $i $lastUserIdx)))) -}}
<think>{{ .Thinking }}</think>
{{ end -}}
{{ if .Content }}{{ .Content }}
{{- else if .ToolCalls }}<tool_call>
{{ range .ToolCalls }}{"name": "{{ .Function.Name }}", "arguments": {{ .Function.Arguments }}}
{{ end }}</tool_call>
{{- end }}{{ if not $last }}<|im_end|>
{{ end }}
{{- else if eq .Role "tool" }}<|im_start|>user
<tool_response>
{{ .Content }}
</tool_response><|im_end|>
{{ end }}
{{- if and (ne .Role "assistant") $last }}<|im_start|>assistant
{{ if and $.IsThinkSet (not $.Think) -}}
<think>

</think>

{{ end -}}
{{ end }}
{{- end }}"###;

    fn profile() -> GenerationProfile {
        GenerationProfile {
            channel_id: None,
            capabilities: None,
            chat_token_limit: None,
            provider: "ollama".into(),
            endpoint: "http://127.0.0.1:11435".into(),
            model: MODEL.into(),
            model_digest: MANIFEST.into(),
            config_revision: 1,
            temperature: 0.0,
            context_tokens: 8192,
        }
    }
    fn show() -> Value {
        json!({"template": VERIFIED_TEMPLATE, "details": {"format":"gguf", "family":"qwen3", "quantization_level":"Q4_K_M"}, "model_info": {"general.architecture":"qwen3", "tokenizer.ggml.model":"gpt2", "qwen3.context_length":40960}, "capabilities":["completion","tools","thinking"]})
    }
    #[test]
    fn admitted_manifest_and_real_template_match() {
        let mut p = profile();
        validate_show(&p, &show(), RENDERER_RUNTIME).unwrap();
        p.model_digest = format!("sha256:{MANIFEST}");
        for context in [2048, 4096, 8192] {
            p.context_tokens = context;
            validate_profile(&p).unwrap();
        }
    }
    #[test]
    fn weight_digest_or_unverified_model_cannot_impersonate_manifest() {
        let mut p = profile();
        p.model_digest = "7f4030143c1c477224c5434f8272c662a8b042079a0a584f0a27a1684fe2e1fa".into();
        assert!(validate_profile(&p).is_err());
        p = profile();
        p.model = "qwen3.5:4b".into();
        assert!(validate_profile(&p).is_err());
        p = profile();
        p.context_tokens = 8193;
        assert!(validate_profile(&p).is_err());
        p = profile();
        p.context_tokens = 1024;
        assert!(validate_profile(&p).is_err());
    }
    #[test]
    fn changed_template_or_hidden_prompt_fails_closed() {
        for (field, value) in [
            ("template", json!(format!("{VERIFIED_TEMPLATE}extra"))),
            ("system", json!("extra instructions")),
            ("messages", json!([{"role":"user","content":"extra"}])),
        ] {
            let mut body = show();
            body[field] = value;
            assert!(validate_show(&profile(), &body, RENDERER_RUNTIME).is_err());
        }
    }
    #[test]
    fn missing_template_or_wrong_tokenizer_fails_closed() {
        let mut body = show();
        body.as_object_mut().unwrap().remove("template");
        assert!(validate_show(&profile(), &body, RENDERER_RUNTIME).is_err());
        let mut body = show();
        body["model_info"]["tokenizer.ggml.model"] = json!("unknown");
        assert!(validate_show(&profile(), &body, RENDERER_RUNTIME).is_err());
    }

    #[test]
    fn real_4b_renderer_requires_exact_runtime_and_manifest() {
        let mut p = profile();
        p.model = MODEL_4B.into();
        p.model_digest = MANIFEST_4B.into();
        let body = json!({"template":"{{ .Prompt }}", "requires":"0.17.1", "details":{"format":"gguf","family":"qwen35","quantization_level":"Q4_K_M"},"model_info":{"general.architecture":"qwen35","tokenizer.ggml.model":"gpt2","qwen35.context_length":262144},"capabilities":["completion","vision","tools","thinking"]});
        validate_show(&p, &body, RENDERER_RUNTIME).unwrap();
        for version in ["", "0.17.1", "0.35.0", "0.35.2", "0.35.1-custom"] {
            assert!(validate_show(&p, &body, version).is_err());
        }
        let mut changed = body.clone();
        changed["template"] = json!("{{ .Prompt }}extra");
        assert!(validate_show(&p, &changed, RENDERER_RUNTIME).is_err());
        p.model_digest = MANIFEST.into();
        assert!(validate_show(&p, &body, RENDERER_RUNTIME).is_err());
    }
}

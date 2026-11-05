//! Provider-agnostic transcript summary. Text leaves the device only through the
//! already-configured generation channel and its existing consent gate.
use crate::{AppError, Result, WorkspaceIdentity};
use pkms_core::{GenerationProfile, VoiceSummaryMode};
use pkms_inference::remote;

pub fn summarize(
    identity: &WorkspaceIdentity,
    profile: &GenerationProfile,
    transcript: &str,
    mode: VoiceSummaryMode,
) -> Result<String> {
    let (instruction, user_prefix) = match mode {
        VoiceSummaryMode::Summary => (
            "你是个人知识库的语音笔记助手。只输出简洁的中文摘要，保留事实、数字和未决问题；不要编造转写中没有的内容。",
            "请总结以下语音转写：\n\n",
        ),
        VoiceSummaryMode::Actions => (
            "你是个人知识库的语音笔记助手。只输出转写中明确出现的待办事项，每行一条，以“- ”开头；没有待办时输出“没有明确待办”。不要编造任务。",
            "请从以下语音转写中提取待办：\n\n",
        ),
    };
    if transcript.trim().is_empty() {
        return Err(AppError::InvalidInput("voice_transcript_empty"));
    }
    let request = remote::Request {
        chat_token_limit: profile.chat_token_limit,
        model: profile.model.clone(),
        messages: vec![
            remote::Message::System {
                text: instruction.into(),
            },
            remote::Message::User {
                text: format!("{user_prefix}{transcript}"),
            },
        ],
        tools: vec![],
        max_output_tokens: 1_024.min(profile.context_tokens / 2),
        temperature: Some(profile.temperature),
        output_format: None,
    };
    let completion = crate::inference_remote::generate_with_limits(
        identity,
        profile,
        request,
        remote::Limits::default(),
        &mut || Ok(()),
    )?;
    if completion.finish != remote::Finish::Stop || completion.text.trim().is_empty() {
        return Err(AppError::InvalidInput("model_incomplete"));
    }
    Ok(completion.text.trim().to_owned())
}

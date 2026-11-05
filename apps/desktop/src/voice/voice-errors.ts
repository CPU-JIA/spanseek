import { voiceFailureReason } from './api';

export function voiceErrorText(error: unknown, t: (zh: string, en: string) => string) {
  const reason = voiceFailureReason(error);
  if (!reason) return t('操作失败，请重试。', 'The operation failed. Try again.');
  return voiceErrorTextByCode(reason, t);
}

export function voiceErrorTextByCode(reason: string, t: (zh: string, en: string) => string) {
  switch (reason) {
    case 'voice_audio_invalid':
      return t(
        '无法识别这个音频格式。支持 WebM、Ogg、WAV、M4A、MP3。',
        'Unsupported audio format. WebM, Ogg, WAV, M4A and MP3 are supported.',
      );
    case 'voice_audio_too_large':
      return t(
        '音频超过 256 MiB 或转写上传超过 25 MiB。',
        'The audio exceeds 256 MiB, or the upload exceeds 25 MiB.',
      );
    case 'voice_duration_too_long':
      return t(
        '录音超过 30 分钟，或导入音频超过 4 小时。',
        'Recordings are limited to 30 minutes and imports to 4 hours.',
      );
    case 'voice_title_invalid':
      return t('标题需为 1–120 个字符。', 'The title must be 1–120 characters.');
    case 'voice_transcript_empty':
      return t(
        '先转写或输入文本，再存入收件箱。',
        'Transcribe or type text before saving to the inbox.',
      );
    case 'voice_transcript_too_long':
      return t('转写文本超过 20 万字符。', 'The transcript exceeds 200,000 characters.');
    case 'asr_not_configured':
      return t('尚未配置转写服务。', 'No transcription service is configured.');
    case 'asr_credential_missing':
      return t(
        '缺少 API 密钥。请在设置的转写服务中保存密钥。',
        'The API key is missing. Save it in Settings → Transcription service.',
      );
    case 'asr_auth_failed':
      return t(
        '转写服务拒绝了密钥。请检查 API 密钥和端点。',
        'The transcription service rejected the key. Check the key and endpoint.',
      );
    case 'asr_rate_limited':
      return t(
        '转写服务限流，请稍后重试。音频仍已保存。',
        'The transcription service is rate-limiting. Retry later; the audio is still saved.',
      );
    case 'asr_service_unavailable':
      return t(
        '转写服务暂时不可用。音频仍已保存，可稍后重试。',
        'The transcription service is unavailable. The audio is saved; retry later.',
      );
    case 'asr_timeout':
      return t(
        '转写超时。音频仍已保存，可重试。',
        'Transcription timed out. The audio is saved; retry when ready.',
      );
    case 'asr_network_error':
      return t(
        '无法连接转写服务。音频仍已保存，可检查网络后重试。',
        'Could not reach the transcription service. The audio is saved; check the network and retry.',
      );
    case 'asr_invalid_response':
      return t(
        '转写服务返回了无法识别的内容。',
        'The transcription service returned an unexpected response.',
      );
    case 'asr_endpoint_invalid':
      return t(
        '端点必须是 HTTPS，或本机回环 HTTP 地址。',
        'The endpoint must be HTTPS, or a loopback HTTP address.',
      );
    case 'asr_settings_incomplete':
      return t('端点与模型需要同时填写。', 'Endpoint and model must be filled together.');
    case 'asr_already_running':
      return t('这条录音正在转写，请稍候。', 'This recording is already being transcribed.');
    case 'asr_interrupted':
      return t(
        '上次转写被中断。音频仍在，可重试。',
        'The last transcription was interrupted. The audio is intact; retry.',
      );
    case 'asr_empty_transcript':
      return t(
        '没有识别到文字。可重试或手工输入。',
        'No speech was recognized. Retry or type the text manually.',
      );
    case 'sync_not_configured':
      return t(
        '尚未配置同步。请先在设置中连接同步服务器并配对。',
        'Sync is not configured. Connect and pair a sync server first.',
      );
    case 'sync_workspace_missing':
      return t('当前同步尚未绑定知识库。', 'The sync profile has no workspace.');
    case 'voice_sync_blob_invalid':
      return t('同步音频数据不完整，请重试。', 'The synced audio blob is incomplete. Try again.');
    case 'voice_sync_hash_mismatch':
      return t(
        '同步音频校验失败，已拒绝导入。',
        'The synced audio failed its hash check and was not imported.',
      );
    case 'model_profile_missing':
      return t(
        '尚未配置生成模型。请先在“设置 → 模型与检索”中配置后再生成摘要。',
        'No generation model is configured. Set one in Settings → Models before generating a summary.',
      );
    case 'model_key_missing':
      return t('生成模型缺少 API 密钥。', 'The generation model has no API key.');
    case 'model_incomplete':
      return t(
        '模型返回内容不完整，请重试。',
        'The model returned an incomplete response. Try again.',
      );
    case 'remote_consent_required':
      return t(
        '尚未同意把音频发送到远程服务。请在“设置 → 检索”中确认远程数据知情同意，或改用本机地址。',
        'Remote audio consent is not confirmed. Confirm it in Settings → Retrieval, or use a loopback endpoint.',
      );
    default:
      return t('操作失败，请重试。', 'The operation failed. Try again.');
  }
}

import { inferenceErrorMessage } from './inference-errors';

export function channelErrorMessage(code: string, t: (zh: string, en: string) => string): string {
  const messages: Record<string, string> = {
    channel_name_invalid: t(
      '请填写 1–120 字的渠道名称。',
      'Enter a channel name of 1–120 characters.',
    ),
    channel_protocol_invalid: t(
      '请选择 OpenAI、Anthropic 或 Gemini 协议。',
      'Choose OpenAI, Anthropic, or Gemini.',
    ),
    channel_endpoint_invalid: t(
      '请填写完整的 HTTPS API 根地址，不包含密钥、查询参数或片段。',
      'Enter an HTTPS API base URL without credentials, query parameters, or fragments.',
    ),
    channel_cloudflare_invalid: t(
      '请填写有效的 Cloudflare Account ID，并核对下方传输设置。Responses 与结构化回答须使用非流式 JSON。',
      'Enter a valid Cloudflare Account ID and check transport settings. Responses and structured answers require non-streaming JSON.',
    ),
    channel_transport_invalid: t(
      '请明确选择文本与结构化回答的传输方式。',
      'Choose text and structured-answer transports explicitly.',
    ),
    channel_model_limit: t(
      '每个渠道最多保存 128 个模型。',
      'A channel can contain up to 128 models.',
    ),
    channel_model_invalid: t(
      '模型 ID 不能为空、包含控制字符或重复；最多 256 个 UTF-8 字节。',
      'Model IDs must be nonempty, unique, contain no control characters, and fit within 256 UTF-8 bytes.',
    ),
    channel_model_parameters_invalid: t(
      '上下文须为 256–2,097,152 的整数，温度须为 0–2。请按模型实际能力填写。',
      'Context must be an integer from 256–2,097,152 and temperature must be 0–2. Use the model’s actual capabilities.',
    ),
    channel_model_budget_invalid: t(
      '输出预算字段只适用于 OpenAI Chat Completions。',
      'The output budget field only applies to OpenAI Chat Completions.',
    ),
    channel_conflict: t(
      '配置已有更新，本次输入已保留。重新读取后核对，或把草稿另存为新渠道。',
      'The configuration changed. Your input is retained. Reload to compare, or save the draft as a new channel.',
    ),
    channel_missing: t(
      '此渠道已被移除，本次输入仍保留。请重新读取渠道列表。',
      'This channel was removed. Your input is retained. Reload the channel list.',
    ),
    channel_active_delete: t(
      '此渠道正在使用。移除时须明确停用当前模型。',
      'This channel is in use. Confirm that removing it will disable the current model.',
    ),
    channel_read_failed: t(
      '渠道读取失败，请重试。已有草稿保持不变。',
      'Channels could not be loaded. Retry; existing drafts are unchanged.',
    ),
    channel_write_failed: t(
      '配置未保存成功，输入已保留。请检查连接后重试。',
      'Configuration was not saved. Your input is retained. Check the connection and retry.',
    ),
    channel_catalog_mismatch: t(
      '返回的模型列表属于其他渠道版本，已忽略。请重新获取。',
      'The model list belongs to a different channel revision and was ignored. Fetch it again.',
    ),
    channel_catalog_failed: t(
      '未能获取模型列表。请检查端点和密钥；也可以继续手动添加模型 ID。',
      'The model list could not be fetched. Check the endpoint and key, or add a model ID manually.',
    ),
    model_discovery_unsupported: t(
      '此服务商不支持模型发现。请手动添加服务商提供的模型 ID。',
      'This provider does not support model discovery. Add the model ID supplied by your provider manually.',
    ),
    model_response_invalid: t(
      '服务商返回的数据格式无效。请核对 API 根地址、协议和模型 ID 后重试。',
      'The provider returned an invalid response. Check the API base URL, protocol, and model ID before retrying.',
    ),
    model_response_limit: t(
      '服务商返回的模型列表超出可处理范围，未采用不完整结果。请手动添加所需模型 ID。',
      'The provider’s model list exceeds the supported limit. No partial list was accepted; add the model IDs you need manually.',
    ),
    model_unsupported_output: t(
      '此渠道无法提供所需输出。请核对结构化证据回答能力与高级传输设置，或切换支持该能力的模型。',
      'This channel cannot provide the required output. Check structured evidence-answer support and advanced transport settings, or choose a model that supports them.',
    ),
    channel_credential_failed: t(
      '密钥操作未完成。请检查系统凭据库后重试，密钥需要重新输入。',
      'The key operation did not complete. Check the system vault and re-enter the key to retry.',
    ),
    channel_verification_failed: t(
      '模型测试未通过。请核对协议、模型 ID 与传输方式后再试。',
      'The model test did not pass. Check the protocol, model ID, and transport before retrying.',
    ),
  };
  return messages[code] ?? inferenceErrorMessage(code, t);
}

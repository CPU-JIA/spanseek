import type { GenerationChannelModel, GenerationChannelSpec } from '@pkms/contracts';

export type ChannelTemplate =
  'openai' | 'anthropic' | 'gemini' | 'openrouter' | 'deepseek' | 'cloudflare' | 'local' | 'custom';
export const channelTemplates: {
  id: ChannelTemplate;
  name: string;
  provider: string;
  endpoint: string;
}[] = [
  { id: 'openai', name: 'OpenAI', provider: 'openai_chat', endpoint: 'https://api.openai.com/v1' },
  {
    id: 'anthropic',
    name: 'Anthropic',
    provider: 'anthropic',
    endpoint: 'https://api.anthropic.com/v1',
  },
  {
    id: 'gemini',
    name: 'Gemini',
    provider: 'gemini',
    endpoint: 'https://generativelanguage.googleapis.com/v1beta',
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    provider: 'openai_chat',
    endpoint: 'https://openrouter.ai/api/v1',
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    provider: 'openai_chat',
    endpoint: 'https://api.deepseek.com',
  },
  { id: 'cloudflare', name: 'Cloudflare Workers AI', provider: 'openai_chat', endpoint: '' },
  {
    id: 'local',
    name: '本机 OpenAI 兼容（LM Studio / llama.cpp）',
    provider: 'openai_chat',
    endpoint: 'http://127.0.0.1:11436/v1',
  },
  { id: 'custom', name: '', provider: 'openai_chat', endpoint: '' },
];

export function createChannelSpec(template: ChannelTemplate): GenerationChannelSpec {
  const selected = channelTemplates.find((item) => item.id === template)!;
  return {
    name: selected.name,
    provider: selected.provider,
    endpoint: selected.endpoint,
    models: [],
    capabilities: {
      text_transport: 'stream',
      structured_transport:
        template === 'cloudflare' ? 'json' : template === 'local' ? null : 'stream',
    },
    preset: template === 'cloudflare' ? 'cloudflare_workers_ai' : null,
  };
}

export function cloudflareEndpoint(accountId: string): string | null {
  const account = accountId.trim();
  return /^[a-f\d]{32}$/i.test(account)
    ? `https://api.cloudflare.com/client/v4/accounts/${account}/ai/v1`
    : null;
}

export function cloudflareAccount(endpoint: string): string {
  return (
    /^https:\/\/api\.cloudflare\.com\/client\/v4\/accounts\/([a-f\d]{32})\/ai\/v1\/?$/i.exec(
      endpoint,
    )?.[1] ?? ''
  );
}

export function protocolFamily(provider: string): 'openai' | 'anthropic' | 'gemini' {
  return provider === 'anthropic' || provider === 'gemini' ? provider : 'openai';
}

export function channelProtocolChange(
  spec: GenerationChannelSpec,
  provider: string,
): GenerationChannelSpec {
  const next = structuredClone(spec);
  next.provider = provider;
  if (provider !== 'openai_chat')
    next.models = next.models.map(({ chat_token_limit: _old, ...model }) => model);
  if (next.preset === 'cloudflare_workers_ai') {
    if (provider !== 'openai_chat' && provider !== 'openai_responses') next.preset = null;
    else {
      next.capabilities = {
        text_transport: provider === 'openai_responses' ? 'json' : 'stream',
        structured_transport: 'json',
      };
    }
  }
  return next;
}

export function newChannelModel(model: string, template?: ChannelTemplate): GenerationChannelModel {
  return {
    model: model.trim(),
    context_tokens: 65536,
    temperature: 0,
    ...(template === 'deepseek' || template === 'cloudflare'
      ? { chat_token_limit: 'max_tokens' as const }
      : {}),
  };
}

export function validateChannelSpec(spec: GenerationChannelSpec): string | null {
  if (!spec.name.trim() || [...spec.name].length > 120 || /[\u0000-\u001f\u007f]/.test(spec.name))
    return 'channel_name_invalid';
  if (!['openai_chat', 'openai_responses', 'anthropic', 'gemini'].includes(spec.provider))
    return 'channel_protocol_invalid';
  try {
    const endpoint = new URL(spec.endpoint);
    const loopbackHttp =
      endpoint.protocol === 'http:' &&
      (endpoint.hostname === '127.0.0.1' || endpoint.hostname === '[::1]');
    if (
      (endpoint.protocol !== 'https:' && !loopbackHttp) ||
      endpoint.username ||
      endpoint.password ||
      endpoint.search ||
      endpoint.hash ||
      spec.endpoint.trim() !== spec.endpoint
    )
      return 'channel_endpoint_invalid';
  } catch {
    return 'channel_endpoint_invalid';
  }
  if (
    spec.preset === 'cloudflare_workers_ai' &&
    (!cloudflareAccount(spec.endpoint) ||
      !['openai_chat', 'openai_responses'].includes(spec.provider) ||
      spec.capabilities.structured_transport === 'stream' ||
      (spec.provider === 'openai_responses' && spec.capabilities.text_transport !== 'json'))
  )
    return 'channel_cloudflare_invalid';
  if (
    !['stream', 'json'].includes(spec.capabilities.text_transport) ||
    ![null, 'stream', 'json'].includes(spec.capabilities.structured_transport)
  )
    return 'channel_transport_invalid';
  if (spec.models.length > 128) return 'channel_model_limit';
  const seen = new Set<string>();
  for (const model of spec.models) {
    if (
      !model.model.trim() ||
      model.model !== model.model.trim() ||
      new TextEncoder().encode(model.model).length > 256 ||
      /[\u0000-\u001f\u007f]/.test(model.model) ||
      seen.has(model.model)
    )
      return 'channel_model_invalid';
    seen.add(model.model);
    if (
      !Number.isInteger(model.context_tokens) ||
      model.context_tokens < 256 ||
      model.context_tokens > 2097152 ||
      !Number.isFinite(model.temperature) ||
      model.temperature < 0 ||
      model.temperature > 2
    )
      return 'channel_model_parameters_invalid';
    if (
      model.chat_token_limit != null &&
      (spec.provider !== 'openai_chat' ||
        !['max_tokens', 'max_completion_tokens'].includes(model.chat_token_limit))
    )
      return 'channel_model_budget_invalid';
  }
  return null;
}

export function channelRequestPreview(spec: GenerationChannelSpec): string {
  const root = spec.endpoint.replace(/\/+$/, '');
  const path =
    spec.provider === 'anthropic'
      ? '/messages'
      : spec.provider === 'openai_responses'
        ? '/responses'
        : spec.provider === 'gemini'
          ? `/models/{model}:${spec.capabilities.text_transport === 'stream' ? 'streamGenerateContent?alt=sse' : 'generateContent'}`
          : '/chat/completions';
  return root ? `${root}${path}` : '';
}

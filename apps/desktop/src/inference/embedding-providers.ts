import type { EmbeddingProvider } from '@pkms/contracts';

/**
 * Embedding providers this build can call, with the parameters verified against
 * each provider's own documentation on 2026-10-15. Provider request shapes are
 * implemented and tested in the Rust inference transport.
 */
export type EmbeddingModelOption = {
  model: string;
  /** Vector size the model returns unless a truncation is requested. */
  dimensions: number;
  /** Documented maximum input tokens for one text, when the provider publishes one. */
  inputTokens: number | null;
  /** Official reference price per million input tokens, shown for orientation only. */
  price: string | null;
  /** The model accepts Gemini's taskType; otherwise the task belongs in the prompt. */
  taskType: boolean;
  /** The provider has shut this model down, so it cannot be selected. */
  retired: boolean;
};

export type EmbeddingProviderOption = {
  id: EmbeddingProvider;
  label: string;
  labelEn: string;
  /** Base URL taken from the provider's own documentation. */
  endpoint: string;
  endpointHint: string;
  /** The provider documents an output-size (truncation) parameter. */
  truncation: boolean;
  /** The model id is typed by the user (compatible servers). */
  freeModel: boolean;
  models: EmbeddingModelOption[];
};

export const EMBEDDING_PROVIDERS: EmbeddingProviderOption[] = [
  {
    id: 'cloudflare',
    label: 'Cloudflare Workers AI',
    labelEn: 'Cloudflare Workers AI',
    // The account id is part of the path, so the field stays empty and the
    // placeholder shows the full template instead of a half-filled URL.
    endpoint: '',
    endpointHint: 'https://api.cloudflare.com/client/v4/accounts/<ACCOUNT_ID>/ai/v1',
    truncation: false,
    freeModel: false,
    models: [
      {
        model: '@cf/baai/bge-m3',
        dimensions: 1024,
        inputTokens: null,
        price: '$0.012',
        taskType: false,
        retired: false,
      },
    ],
  },
  {
    id: 'openai',
    label: 'OpenAI',
    labelEn: 'OpenAI',
    endpoint: 'https://api.openai.com/v1',
    endpointHint: 'https://api.openai.com/v1',
    truncation: true,
    freeModel: false,
    models: [
      {
        model: 'text-embedding-3-small',
        dimensions: 1536,
        inputTokens: 8192,
        price: '$0.02',
        taskType: false,
        retired: false,
      },
      {
        model: 'text-embedding-3-large',
        dimensions: 3072,
        inputTokens: 8192,
        price: '$0.13',
        taskType: false,
        retired: false,
      },
    ],
  },
  {
    id: 'gemini',
    label: 'Google Gemini',
    labelEn: 'Google Gemini',
    endpoint: 'https://generativelanguage.googleapis.com/v1beta',
    endpointHint: 'https://generativelanguage.googleapis.com/v1beta',
    truncation: true,
    freeModel: false,
    models: [
      {
        model: 'gemini-embedding-001',
        dimensions: 3072,
        inputTokens: 2048,
        price: '$0.15',
        taskType: true,
        retired: false,
      },
      {
        model: 'gemini-embedding-2',
        dimensions: 3072,
        inputTokens: 8192,
        price: null,
        taskType: false,
        retired: false,
      },
      {
        model: 'text-embedding-004',
        dimensions: 768,
        inputTokens: 2048,
        price: null,
        taskType: true,
        retired: true,
      },
    ],
  },
  {
    id: 'openai_compatible',
    label: 'OpenAI 兼容端点（自建/网关）',
    labelEn: 'OpenAI-compatible endpoint (self-hosted or gateway)',
    endpoint: '',
    endpointHint: 'http://127.0.0.1:11434/v1',
    truncation: true,
    freeModel: true,
    models: [],
  },
];

/** Sizes the providers document for truncation; the native size is the default. */
export const DIMENSION_CHOICES = [256, 512, 768, 1024, 1536, 2048, 3072];

export function providerOption(id: EmbeddingProvider): EmbeddingProviderOption {
  return EMBEDDING_PROVIDERS.find((option) => option.id === id) ?? EMBEDDING_PROVIDERS[0];
}

export function modelOption(
  id: EmbeddingProvider,
  model: string,
): EmbeddingModelOption | undefined {
  return providerOption(id).models.find((option) => option.model === model);
}

export function providerLabel(id: EmbeddingProvider, english: boolean): string {
  const option = providerOption(id);
  return english ? option.labelEn : option.label;
}

/** Only a loopback address may be plain HTTP; anything remote must be HTTPS. */
export function plainHttpAllowed(endpoint: string): boolean {
  if (!endpoint.startsWith('http://')) return true;
  const authority = endpoint.slice('http://'.length).split(/[/?#]/)[0];
  const host = authority.includes('@')
    ? authority.slice(authority.lastIndexOf('@') + 1)
    : authority;
  const name = host.startsWith('[') ? host.slice(1, host.indexOf(']')) : host.split(':')[0];
  return name === 'localhost' || name === '127.0.0.1' || name === '::1';
}

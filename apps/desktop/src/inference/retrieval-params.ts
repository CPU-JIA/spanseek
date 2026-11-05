import type { RetrievalParams } from '@pkms/contracts';

/** Mirrors the Rust defaults so an unconfigured library behaves identically. */
export const DEFAULT_RETRIEVAL_PARAMS: RetrievalParams = {
  top_k: 8,
  citation_count: 5,
  minimum_cosine: null,
  semantic_enabled: true,
  remote_consent: false,
  cost_per_million_input: null,
  cost_per_million_output: null,
  answer_timeout_seconds: null,
};
export const MAX_TOP_K = 50;
export const MAX_CITATION_COUNT = 20;
/** Typo guard for a user-entered reference price; it is not a price list. */
export const MAX_COST_PER_MILLION = 1_000_000;
/** Bounds for the optional per-library answer budget, in seconds. */
export const MIN_ANSWER_TIMEOUT_SECONDS = 10;
export const MAX_ANSWER_TIMEOUT_SECONDS = 3600;

/**
 * Whether a minimum-similarity floor is actually applied.
 *
 * The core omits `minimum_cosine` when it is unset, so the interface receives
 * `undefined` as well as `null`. Both mean "off" and must not be rendered as an
 * enabled filter with a suggested value in it.
 */
export function minimumCosineEnabled(value: number | null | undefined): boolean {
  return value !== null && value !== undefined;
}
/** Keeps every field inside the range the core accepts, and never lets the
 *  citation count exceed the candidate width. */
export function clampParams(params: RetrievalParams): RetrievalParams {
  const topK = Math.min(
    MAX_TOP_K,
    Math.max(1, Math.round(params.top_k) || DEFAULT_RETRIEVAL_PARAMS.top_k),
  );
  const citations = Math.min(
    MAX_CITATION_COUNT,
    topK,
    Math.max(1, Math.round(params.citation_count) || DEFAULT_RETRIEVAL_PARAMS.citation_count),
  );
  const cosine = params.minimum_cosine;
  return {
    top_k: topK,
    citation_count: citations,
    minimum_cosine:
      cosine === null || cosine === undefined || !Number.isFinite(cosine)
        ? null
        : Math.min(1, Math.max(-1, cosine)),
    semantic_enabled: params.semantic_enabled !== false,
    remote_consent: params.remote_consent === true,
    cost_per_million_input: clampPrice(params.cost_per_million_input),
    cost_per_million_output: clampPrice(params.cost_per_million_output),
    answer_timeout_seconds: clampAnswerTimeout(params.answer_timeout_seconds),
  };
}

/**
 * An absent budget stays absent so the core applies the provider default; a
 * present one is kept inside the range the core accepts rather than silently
 * replaced.
 */
export function clampAnswerTimeout(value: number | null | undefined): number | null {
  if (value === null || value === undefined || !Number.isFinite(value)) return null;
  return Math.min(
    MAX_ANSWER_TIMEOUT_SECONDS,
    Math.max(MIN_ANSWER_TIMEOUT_SECONDS, Math.round(value)),
  );
}

/** An absent or unusable price stays absent: the app never invents one. */
export function clampPrice(value: number | null | undefined): number | null {
  if (value === null || value === undefined || !Number.isFinite(value)) return null;
  return Math.min(MAX_COST_PER_MILLION, Math.max(0, value));
}

/**
 * Cosine distributions differ per model and dimension, so this is a per-model
 * reference floor rather than a universal constant. Only the default profile has
 * measured calibration; every other model keeps a conservative placeholder until
 * a model-specific calibration is recorded.
 */
export function suggestedCosine(model: string | null, dimensions: number | null): number {
  const key = (model ?? '').toLowerCase();
  if (key.includes('bge-m3')) return 0.35;
  if (key.includes('text-embedding-3')) return 0.3;
  if (key.includes('text-embedding-004')) return 0.4;
  if (key.includes('gemini-embedding-001')) return 0.35;
  if (key.includes('gemini-embedding-2')) return 0.35;
  if (dimensions !== null && dimensions >= 1024) return 0.35;
  return 0.35;
}

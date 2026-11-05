import type { CleaningParams, RetrievalParams } from '@pkms/contracts';

function optionalNumber(value: number | null | undefined): number | null {
  return value ?? null;
}

/**
 * Compares the effective retrieval values rather than object identity. The core
 * may omit optional fields, while the saved form keeps explicit nulls, so a
 * strict object comparison would show a false "unsaved changes" state.
 */
export function retrievalParamsEqual(
  left: RetrievalParams | null | undefined,
  right: RetrievalParams | null | undefined,
): boolean {
  if (!left || !right) return false;
  return (
    left.top_k === right.top_k &&
    left.citation_count === right.citation_count &&
    optionalNumber(left.minimum_cosine) === optionalNumber(right.minimum_cosine) &&
    (left.semantic_enabled !== false) === (right.semantic_enabled !== false) &&
    (left.remote_consent === true) === (right.remote_consent === true) &&
    optionalNumber(left.cost_per_million_input) === optionalNumber(right.cost_per_million_input) &&
    optionalNumber(left.cost_per_million_output) ===
      optionalNumber(right.cost_per_million_output) &&
    optionalNumber(left.answer_timeout_seconds) === optionalNumber(right.answer_timeout_seconds)
  );
}

export function cleaningParamsEqual(
  left: CleaningParams | null | undefined,
  right: CleaningParams | null | undefined,
): boolean {
  if (!left || !right) return false;
  return (
    left.headers_footers === right.headers_footers &&
    left.watermark === right.watermark &&
    left.control_chars === right.control_chars
  );
}

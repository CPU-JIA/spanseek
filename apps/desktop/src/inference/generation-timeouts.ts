/**
 * Mirrors `crates/pkms-core/src/generation_timeouts.rs`.
 *
 * The core resolves these defaults when no explicit budget is set, so the
 * interface shows the same numbers instead of keeping a second, drifting copy.
 * A model that runs on this machine may legitimately take minutes; a remote
 * provider is network-bound and fails faster.
 */
export const LOCAL_PROVIDER = 'ollama';

export type GenerationPurpose = 'answer' | 'chat' | 'embedding';

/** Seconds allowed for one unit of purpose work on provider. */
export function defaultTimeoutSeconds(
  provider: string | null | undefined,
  purpose: GenerationPurpose,
): number {
  if (provider === LOCAL_PROVIDER) return purpose === 'answer' ? 600 : 900;
  if (purpose === 'answer') return 180;
  if (purpose === 'chat') return 300;
  return 120;
}

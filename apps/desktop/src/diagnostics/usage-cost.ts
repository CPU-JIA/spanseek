import type { ModelUsageStats } from '@pkms/contracts';

export type CostPrices = { input: number | null; output: number | null };

/**
 * Estimated spend from measured tokens and the reference prices the user entered
 * for their own provider account. A missing price contributes nothing, and with
 * no price at all there is no estimate rather than a fabricated one.
 */
export function estimateCost(usage: ModelUsageStats, prices: CostPrices): number | null {
  const { input, output } = prices;
  if (input === null && output === null) return null;
  const perMillion = usage.input_tokens * (input ?? 0) + usage.output_tokens * (output ?? 0);
  return perMillion / 1_000_000;
}

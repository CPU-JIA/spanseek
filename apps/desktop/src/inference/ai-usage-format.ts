import type { ChatUsage } from '@pkms/contracts';

export function chatUsageParts(usage: ChatUsage): {
  input: number | null;
  output: number | null;
  total: number | null;
} {
  const input = usage.input_tokens === null ? null : Number(usage.input_tokens);
  const output = usage.output_tokens === null ? null : Number(usage.output_tokens);
  return {
    input,
    output,
    total: input === null && output === null ? null : (input ?? 0) + (output ?? 0),
  };
}

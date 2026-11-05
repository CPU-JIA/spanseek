import type { SourceJob } from '@pkms/contracts';

/** A delayed poll must never undo a confirmed cancel or a newer worker attempt. */
export function latestSourceJob(previous: SourceJob | undefined, incoming: SourceJob): SourceJob {
  if (!previous) return incoming;
  if (incoming.attempt < previous.attempt) return previous;
  if (incoming.attempt > previous.attempt) return incoming;
  if (Date.parse(incoming.updated_at) < Date.parse(previous.updated_at)) return previous;
  const terminal = (status: string) => ['succeeded', 'failed', 'canceled'].includes(status);
  if (terminal(previous.status) && !terminal(incoming.status)) return previous;
  return incoming;
}

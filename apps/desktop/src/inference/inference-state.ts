import type { AnswerRecord, RebuildJob } from '@pkms/contracts';
export function latestAnswer(previous: AnswerRecord | null, incoming: AnswerRecord): AnswerRecord {
  return previous?.id === incoming.id && previous.last_event_seq > incoming.last_event_seq
    ? previous
    : incoming;
}
export function latestRebuild(previous: RebuildJob, incoming: RebuildJob): RebuildJob {
  const terminal = (status: string) => ['completed', 'failed', 'canceled'].includes(status);
  if (terminal(previous.status) && !terminal(incoming.status)) return previous;
  return Date.parse(previous.updated_at) > Date.parse(incoming.updated_at) ? previous : incoming;
}

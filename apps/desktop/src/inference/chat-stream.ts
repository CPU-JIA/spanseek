import type { ChatRunEvent, ChatRunStatus } from '@pkms/contracts';

export type ChatStream = { seq: number; text: string; status: ChatRunStatus };
export const emptyChatStream = (): ChatStream => ({ seq: 0, text: '', status: 'queued' });
export const chatActive = (status: ChatRunStatus) => status === 'queued' || status === 'running';

/** Reconstruct from durable events, never from an unpaired message/run snapshot. */
export function appendChatEvents(previous: ChatStream, events: ChatRunEvent[]): ChatStream {
  let current = previous;
  for (const event of events) {
    if (event.seq <= current.seq) continue;
    if (event.seq !== current.seq + 1)
      throw new Error('Chat event gap; retry from the last saved sequence');
    if (!chatActive(current.status)) throw new Error('Chat event after terminal state');
    if (event.kind === 'text_delta' && (event.text_delta === null || event.status !== 'running'))
      throw new Error('Invalid chat text event');
    current = {
      seq: event.seq,
      text: current.text + (event.kind === 'text_delta' ? event.text_delta : ''),
      status: event.status,
    };
  }
  return current;
}

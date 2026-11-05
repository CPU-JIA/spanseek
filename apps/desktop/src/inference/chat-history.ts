import type { ChatConversation, ChatMessage, ChatMessagePage, ChatRun } from '@pkms/contracts';
import type { ChatStream } from './chat-stream';

export type ChatStreamSeed = { run: ChatRun; stream: ChatStream };

/** Only use the atomic page snapshot: separately fetched text/run can lose a delta. */
export function chatPageSeed(page: ChatMessagePage): ChatStreamSeed | null {
  const run = page.latest_run;
  if (!run) return null;
  const last = page.items.at(-1);
  if (
    !last ||
    last.id !== run.assistant_message_id ||
    last.run_id !== run.id ||
    last.role !== 'assistant' ||
    last.run_status !== run.status ||
    last.conversation_id !== run.conversation_id ||
    run.conversation_id !== page.conversation.id
  ) {
    throw new Error('Chat snapshot does not match its latest response');
  }
  return { run, stream: { seq: run.last_event_seq, text: last.content, status: run.status } };
}

export function prependChatMessages(current: ChatMessage[], older: ChatMessage[]): ChatMessage[] {
  if (!older.length) return current;
  const first = current[0];
  if (
    first &&
    older.some(
      (item) => item.conversation_id !== first.conversation_id || item.ordinal >= first.ordinal,
    )
  )
    throw new Error('Chat history page overlaps or belongs to another conversation');
  return [...older, ...current];
}

export function appendChatConversations(
  current: ChatConversation[],
  older: ChatConversation[],
): ChatConversation[] {
  const ids = new Set(current.map((item) => item.id));
  return [...current, ...older.filter((item) => !ids.has(item.id))];
}

/** A historical partial reply never becomes applicable merely because a newer run exists. */
export function canSaveChatReply(
  item: ChatMessage,
  latestMessageId: string | undefined,
  liveRun: ChatRun | null,
  stream: ChatStream,
): boolean {
  if (item.role !== 'assistant') return false;
  if (item.id !== latestMessageId) return item.run_status === 'completed';
  return liveRun?.id === item.run_id && stream.status === 'completed';
}

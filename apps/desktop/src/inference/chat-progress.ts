import type { ChatProgress, ChatRunStatus, WorkspaceIdentity } from '@pkms/contracts';

export function latestChatProgress(
  previous: ChatProgress | null,
  incoming: ChatProgress,
  identity: WorkspaceIdentity,
  eventIdentity: WorkspaceIdentity,
  runId: string | null,
  status: ChatRunStatus,
): ChatProgress | null {
  if (
    (status !== 'queued' && status !== 'running') ||
    incoming.run_id !== runId ||
    identity.local_instance_id !== eventIdentity.local_instance_id ||
    identity.workspace_id !== eventIdentity.workspace_id
  )
    return previous;
  if (
    previous?.run_id === runId &&
    (incoming.elapsed_ms < previous.elapsed_ms ||
      incoming.model_calls < previous.model_calls ||
      incoming.tool_calls < previous.tool_calls)
  )
    return previous;
  return incoming;
}

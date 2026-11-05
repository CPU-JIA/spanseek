import type { AgentOptions, AgentProgress, WorkspaceIdentity } from '@pkms/contracts';
import { defaultTimeoutSeconds } from './generation-timeouts.ts';

/**
 * Run options for the evidence agent. The total budget follows the selected
 * provider so a local model is not cut off at the remote-provider default;
 * every other field is a fixed planning bound.
 */
export function agentDefaults(provider: string | null | undefined): AgentOptions {
  return {
    max_model_calls: 4,
    max_tool_calls: 8,
    max_evidence: 32,
    total_timeout_seconds: defaultTimeoutSeconds(provider, 'answer'),
  };
}

export function latestAgentProgress(
  previous: AgentProgress | null,
  incoming: AgentProgress,
  identity: WorkspaceIdentity,
  eventIdentity: WorkspaceIdentity,
  answerId: string | null,
): AgentProgress | null {
  if (
    identity.local_instance_id !== eventIdentity.local_instance_id ||
    identity.workspace_id !== eventIdentity.workspace_id ||
    incoming.answer_id !== answerId
  )
    return previous;
  if (
    previous?.answer_id === incoming.answer_id &&
    (incoming.elapsed_ms < previous.elapsed_ms ||
      incoming.model_calls < previous.model_calls ||
      incoming.tool_calls < previous.tool_calls)
  )
    return previous;
  return incoming;
}

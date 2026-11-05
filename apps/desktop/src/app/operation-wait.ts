/** Workspace bootstrap operations are polled briefly; cancellation has its own settle budget. */
export const WORKSPACE_OPERATION_POLL_MS = 200;
export const WORKSPACE_OPERATION_MAX_MS = 10 * 60_000;
export const WORKSPACE_CANCEL_SETTLE_MS = 15_000;

export function operationWaitDeadline(startedAt: number, cancelRequestedAt: number | null): number {
  if (cancelRequestedAt !== null) return cancelRequestedAt + WORKSPACE_CANCEL_SETTLE_MS;
  return startedAt + WORKSPACE_OPERATION_MAX_MS;
}

/**
 * The kernel answers `backend_starting` while it opens the app state. The window
 * used to re-ask every 250 ms forever: a machine that was never going to start
 * (missing sidecar, blocked disk) kept the IPC busy and never told the user
 * anything. The delay now doubles to a ceiling, a hidden window waits much
 * longer, and a spent budget becomes a visible failure with a retry button.
 */
export const STARTUP_RETRY_BASE_MS = 250;
export const STARTUP_RETRY_CEILING_MS = 4000;
/** A hidden window has nobody waiting on it, so it re-asks rarely. */
export const STARTUP_RETRY_HIDDEN_MS = 8000;
/** 250 ms doubling to 4 s: about 40 s of visible waiting before we give up. */
export const STARTUP_RETRY_LIMIT = 14;

/** Delay before the next `app_state` read, or `null` when the budget is spent. */
export function startupRetryDelay(attempt: number, hidden: boolean): number | null {
  if (!Number.isFinite(attempt) || attempt < 0) return STARTUP_RETRY_BASE_MS;
  if (attempt >= STARTUP_RETRY_LIMIT) return null;
  if (hidden) return STARTUP_RETRY_HIDDEN_MS;
  return Math.min(STARTUP_RETRY_BASE_MS * 2 ** attempt, STARTUP_RETRY_CEILING_MS);
}

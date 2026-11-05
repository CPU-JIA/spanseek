/**
 * Cadence for the task center. The decision lives here, so the components only
 * subscribe to window visibility and feed observation counts in; a schedule that
 * decides "poll again in 1.6 s" for hours is the defect this replaces.
 */

/** Visible window with a queued/running operation or job: the V1.6 cadence. */
export const POLL_ACTIVE_MS = 1600;
/** Visible window with nothing queued or running: stop paying the fast cadence. */
export const POLL_IDLE_MS = 10_000;
/** Hidden window: no fast cadence, only a slow heartbeat so long work still lands. */
export const POLL_HIDDEN_MS = 30_000;
/** Upper bound for the failure backoff so a broken backend is still retried. */
export const POLL_MAX_MS = 60_000;
/** The source sweep covers active jobs only, so it keeps its own active cadence. */
export const SOURCE_POLL_ACTIVE_MS = 1800;

export type PollInputs = {
  /** `document.visibilityState !== 'hidden'`. Hidden windows stop the fast cadence. */
  visible: boolean;
  /** `operations_list` rows still queued or running. */
  pendingOperations: number;
  /** `jobs_list` rows still queued or running. */
  activeJobs: number;
  /** Consecutive poll cycles where at least one source rejected. */
  failures: number;
};

/** The source sweep has no operations to count. */
export type SourcePollInputs = Omit<PollInputs, 'pendingOperations'>;

/** What a visibility change should do with the timer that is currently armed. */
export type VisibilityPlan = {
  /** Poll as soon as the window is visible again instead of waiting out a delay. */
  pollNow: boolean;
  /** Delay to arm when the change does not poll immediately. */
  delay: number;
};

const backoff = (base: number, failures: number) =>
  Math.min(base * 2 ** Math.max(0, failures), POLL_MAX_MS);

/** Delay before the next `operations_list` + `jobs_list` poll. */
export function nextPollDelay({
  visible,
  pendingOperations,
  activeJobs,
  failures,
}: PollInputs): number {
  if (!visible) return backoff(POLL_HIDDEN_MS, failures);
  const active = pendingOperations + activeJobs > 0;
  return backoff(active ? POLL_ACTIVE_MS : POLL_IDLE_MS, failures);
}

/** Delay before the next `source_job_get` sweep of the jobs already known to be active. */
export function nextSourcePollDelay({ visible, activeJobs, failures }: SourcePollInputs): number {
  if (!visible) return backoff(POLL_HIDDEN_MS, failures);
  return backoff(activeJobs > 0 ? SOURCE_POLL_ACTIVE_MS : POLL_IDLE_MS, failures);
}

const plan = (visible: boolean, delay: number): VisibilityPlan => ({
  pollNow: visible,
  delay,
});

/**
 * A hidden window drops to the heartbeat; becoming visible polls once right away,
 * because everything read while hidden can already be stale.
 */
export function pollOnVisibilityChange(inputs: PollInputs): VisibilityPlan {
  return plan(inputs.visible, nextPollDelay(inputs));
}

/** Same policy for the active-source-job sweep. */
export function sourcePollOnVisibilityChange(inputs: SourcePollInputs): VisibilityPlan {
  return plan(inputs.visible, nextSourcePollDelay(inputs));
}

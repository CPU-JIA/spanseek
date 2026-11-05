import type { SessionLayout, SessionState, WorkspaceIdentity } from '@pkms/contracts';

/** A pure, idempotent target change. Capture desired values; never toggle on retry. */
export type LayoutChange = (layout: SessionLayout) => SessionLayout;
export type SessionQueueOptions = {
  read: () => Promise<SessionState>;
  save: (baseVersion: number, layout: SessionLayout) => Promise<SessionState>;
  onConfirmed?: (session: SessionState) => void;
};
type Attempt = { baseVersion: number; target: SessionLayout };
type PendingChange = { change: LayoutChange; attempt: Attempt | null };
export type SessionQueueSnapshot = {
  session: SessionState;
  pending: number;
  busy: boolean;
  error: unknown | null;
};

function equal(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (!left || !right || typeof left !== 'object' || typeof right !== 'object') return false;
  if (Array.isArray(left) || Array.isArray(right))
    return (
      Array.isArray(left) &&
      Array.isArray(right) &&
      left.length === right.length &&
      left.every((value, index) => equal(value, right[index]))
    );
  const a = left as Record<string, unknown>;
  const b = right as Record<string, unknown>;
  const keys = Object.keys(a);
  return (
    keys.length === Object.keys(b).length && keys.every((key) => key in b && equal(a[key], b[key]))
  );
}
function sameIdentity(left: WorkspaceIdentity, right: WorkspaceIdentity): boolean {
  return (
    left.local_instance_id === right.local_instance_id && left.workspace_id === right.workspace_id
  );
}
function conflict(error: unknown): boolean {
  return (error as { details?: { code?: string } } | null)?.details?.code === 'app_state_conflict';
}

/**
 * One queue per workspace instance. A rejected flush retains the head operation;
 * a later flush retries it before any subsequently enqueued change. The queue is
 * in-memory: it does not claim to persist unsubmitted layout changes on reload.
 *
 * Reconciliation reapplies an idempotent target to the latest persisted layout,
 * preserving unrelated concurrent fields. For example, ensure a tab exists or
 * set sidebar_visible to a captured boolean. `value => !value` is not a target.
 */
export class SessionQueue {
  private readonly identity: WorkspaceIdentity;
  private readonly options: SessionQueueOptions;
  private readonly pending: PendingChange[] = [];
  private running: Promise<SessionState> | null = null;
  private snapshot: SessionQueueSnapshot;

  constructor(initial: SessionState, options: SessionQueueOptions) {
    this.identity = structuredClone(initial.identity);
    this.options = options;
    this.snapshot = { session: structuredClone(initial), pending: 0, busy: false, error: null };
  }

  getSnapshot = (): SessionQueueSnapshot => structuredClone(this.snapshot);

  enqueue(change: LayoutChange): Promise<SessionState> {
    // Reject programmer mistakes before accepting a new operation into the queue.
    try {
      this.apply(change, this.snapshot.session.layout);
    } catch (error) {
      return Promise.reject(error);
    }
    const entry: PendingChange = { change, attempt: null };
    this.pending.push(entry);
    this.snapshot = { ...this.snapshot, pending: this.pending.length };
    return this.flush().then((session) =>
      // A caller may enqueue during the microtask in which the preceding drain
      // settles. Its returned promise must still cover its own operation.
      this.pending.includes(entry) ? this.flush() : session,
    );
  }

  flush(): Promise<SessionState> {
    if (this.running)
      return this.running.then((session) => (this.pending.length ? this.flush() : session));
    this.snapshot = { ...this.snapshot, busy: true, error: null };
    // Defer execution until running is installed, including synchronous no-op changes.
    this.running = Promise.resolve()
      .then(() => this.drain())
      .catch((error: unknown) => {
        this.snapshot = { ...this.snapshot, error };
        throw error;
      })
      .finally(() => {
        this.running = null;
        this.snapshot = { ...this.snapshot, busy: false, pending: this.pending.length };
      });
    return this.running;
  }

  private apply(change: LayoutChange, base: SessionLayout): SessionLayout {
    const target = structuredClone(change(structuredClone(base)));
    if (!equal(change(structuredClone(target)), target))
      throw new Error(
        '布局变更必须设置明确目标，不能在重试时再次切换。 / Layout changes must be idempotent targets.',
      );
    return target;
  }

  private validate(session: SessionState): void {
    if (
      !session?.identity ||
      !sameIdentity(session.identity, this.identity) ||
      !Number.isSafeInteger(session.state_version) ||
      session.state_version < this.snapshot.session.state_version
    )
      throw new Error(
        '布局回执身份或版本不匹配，待保存操作已保留。 / Session identity or version mismatch.',
      );
  }

  private confirmed(session: SessionState): void {
    this.validate(session);
    const copy = structuredClone(session);
    this.snapshot = { ...this.snapshot, session: copy };
    this.options.onConfirmed?.(structuredClone(copy));
  }

  private complete(): void {
    this.pending.shift();
    this.snapshot = { ...this.snapshot, pending: this.pending.length };
  }

  private async reconcile(entry: PendingChange): Promise<boolean> {
    const current = await this.options.read();
    this.confirmed(current);
    // This also recognizes an accepted change followed by unrelated concurrent edits.
    return equal(this.apply(entry.change, current.layout), current.layout);
  }

  private async drain(): Promise<SessionState> {
    while (this.pending.length) {
      const entry = this.pending[0];
      if (entry.attempt) {
        if (await this.reconcile(entry)) {
          this.complete();
          continue;
        }
        entry.attempt = null;
      }
      let retries = 0;
      for (;;) {
        const base = this.snapshot.session;
        const target = this.apply(entry.change, base.layout);
        if (equal(target, base.layout)) {
          // A cached no-op is not proof that another window has not changed it.
          if (await this.reconcile(entry)) {
            this.complete();
            break;
          }
          continue;
        }
        const attempt = { baseVersion: base.state_version, target };
        entry.attempt = attempt;
        try {
          const result = await this.options.save(attempt.baseVersion, structuredClone(target));
          this.validate(result);
          if (result.state_version !== attempt.baseVersion + 1 || !equal(result.layout, target))
            throw new Error(
              '布局保存回执不匹配，待确认操作已保留。 / Session save receipt mismatch.',
            );
          this.confirmed(result);
          this.complete();
          break;
        } catch (error) {
          if (await this.reconcile(entry)) {
            this.complete();
            break;
          }
          // Conflicts are definitive non-commits; rebase the same target on the
          // fresh session. Other failures stay visible and require a new flush.
          // Bound conflict retries so an active second client cannot monopolize UI.
          if (!conflict(error) || ++retries >= 3) throw error;
          entry.attempt = null;
        }
      }
    }
    return structuredClone(this.snapshot.session);
  }
}

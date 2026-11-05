import type { DesktopCommand, WorkspaceIdentity } from '@pkms/contracts';

type PortableCommand = Extract<
  DesktopCommand,
  {
    kind:
      | 'backup_create'
      | 'backup_verify'
      | 'backup_restore'
      | 'note_export'
      | 'source_export'
      | 'diagnostics_export';
  }
>;
export type PendingAction = {
  id: string;
  identity: WorkspaceIdentity | null;
  command: PortableCommand;
  displayName: string;
  createdAt: string;
};
export type AcceptedAction = { kind: 'job' | 'operation'; id: string };
export interface PendingStorage {
  read(): Promise<PendingAction[]>;
  put(action: PendingAction): Promise<void>;
  remove(id: string): Promise<void>;
}
/**
 * A request the backend refused for good before acceptance. It is dropped from
 * the queue, so it can no longer own a row: the interface names it (what was
 * asked, for which file, when) instead of leaving a bare error line that the
 * user cannot connect to anything.
 */
export type DroppedRequest = {
  id: string;
  kind: string;
  displayName: string;
  createdAt: string;
  message: string;
};
type Snapshot = {
  pending: PendingAction[];
  busy: string[];
  loaded: boolean;
  error: string | null;
  notice: DroppedRequest | null;
};
const errorMessage = (error: unknown) =>
  error instanceof Error ? error.message : 'Request could not be confirmed.';
/**
 * A refusal the backend can no longer change: the request was never accepted, so
 * sending the same input again cannot succeed. An identity change, IO failure or
 * mismatched receipt may follow a commit, so those are not definitive: the queue
 * keeps them for an exact retry.
 */
export const definitiveRejection = (error: unknown, action: PendingAction) => {
  const code = (error as { details?: { code?: string } } | null)?.details?.code;
  return (
    (action.command.kind === 'diagnostics_export' && code === 'app_state_conflict') ||
    (!!code &&
      [
        'invalid_input',
        'unsupported_api',
        'input_unavailable',
        'output_exists',
        'not_found',
        'object_trashed',
      ].includes(code))
  );
};

/**
 * What the action banner should say when a request fails. A definitively
 * refused request is already named by the queue notice, so a second line here
 * would report one failure twice; everything else keeps its own message.
 */
export const actionErrorText = (
  error: unknown,
  action: PendingAction | undefined,
): string | null => (action && definitiveRejection(error, action) ? null : errorMessage(error));

/** Durable acceptance only. The backend owns task execution, results and cancellation. */
export class PendingActions {
  private snapshot: Snapshot = { pending: [], busy: [], loaded: false, error: null, notice: null };
  private listeners = new Set<() => void>();
  private loading: Promise<void> | null = null;
  private storage: PendingStorage;
  private send: (action: PendingAction) => Promise<AcceptedAction>;
  constructor(storage: PendingStorage, send: (action: PendingAction) => Promise<AcceptedAction>) {
    this.storage = storage;
    this.send = send;
  }
  getSnapshot = () => this.snapshot;
  subscribe = (fn: () => void) => {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  };
  private update(patch: Partial<Snapshot>) {
    this.snapshot = { ...this.snapshot, ...patch };
    this.listeners.forEach((fn) => fn());
  }
  load = (): Promise<void> => {
    if (this.snapshot.loaded) return Promise.resolve();
    if (this.loading) return this.loading;
    this.loading = this.storage
      .read()
      .then((pending) => this.update({ pending, loaded: true, error: null }))
      .catch((error) => {
        this.update({ error: errorMessage(error) });
        throw error;
      })
      .finally(() => {
        this.loading = null;
      });
    return this.loading;
  };
  async submit(action: PendingAction): Promise<AcceptedAction> {
    // Freeze the exact intent before the first asynchronous boundary.
    action = structuredClone(action);
    if (action.id !== action.command.payload.command_id)
      throw new Error('请求标识不一致 / Request ID mismatch.');
    await this.load();
    if (this.snapshot.pending.some((entry) => entry.id === action.id))
      throw new Error('请重新确认已有请求 / Check the existing request again.');
    if (this.snapshot.busy.includes(action.id))
      throw new Error('请求正在提交 / Request is already being submitted.');
    this.update({ busy: [...this.snapshot.busy, action.id], error: null });
    try {
      await this.storage.put(structuredClone(action));
      this.update({ pending: [...this.snapshot.pending, structuredClone(action)] });
      return await this.dispatch(action);
    } catch (error) {
      // A definitive rejection is reported as a named notice by `dispatch`, not
      // as a second, unattributed banner line.
      if (!definitiveRejection(error, action)) this.update({ error: errorMessage(error) });
      throw error;
    } finally {
      this.update({ busy: this.snapshot.busy.filter((id) => id !== action.id) });
    }
  }
  async retry(id: string): Promise<AcceptedAction> {
    await this.load();
    const action = this.snapshot.pending.find((entry) => entry.id === id);
    if (!action || this.snapshot.busy.includes(id))
      throw new Error('请求不存在或正在确认 / Request is unavailable or already running.');
    this.update({ busy: [...this.snapshot.busy, id], error: null });
    try {
      return await this.dispatch(action);
    } catch (error) {
      if (!definitiveRejection(error, action)) this.update({ error: errorMessage(error) });
      throw error;
    } finally {
      this.update({ busy: this.snapshot.busy.filter((entry) => entry !== id) });
    }
  }
  private async dispatch(action: PendingAction): Promise<AcceptedAction> {
    let accepted: AcceptedAction;
    try {
      accepted = await this.send(structuredClone(action));
    } catch (error) {
      if (definitiveRejection(error, action)) {
        await this.clear(action.id);
        this.update({
          error: null,
          notice: {
            id: action.id,
            kind: action.command.kind,
            displayName: action.displayName,
            createdAt: action.createdAt,
            message: errorMessage(error),
          },
        });
      }
      throw error;
    }
    // A cleanup failure keeps the exact command available for receipt replay.
    await this.clear(action.id);
    return accepted;
  }
  /** The named rejection was read; stop showing it. */
  dismissNotice = () => this.update({ notice: null });
  private async clear(id: string) {
    await this.storage.remove(id);
    this.update({ pending: this.snapshot.pending.filter((entry) => entry.id !== id), error: null });
  }
}

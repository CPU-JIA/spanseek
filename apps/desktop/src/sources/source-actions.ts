import type { DesktopCommand, DesktopData, WorkspaceIdentity } from '@pkms/contracts';
export type SourceIntent = {
  key: string;
  identity: WorkspaceIdentity;
  command: DesktopCommand;
  expected: DesktopData['kind'];
};
export interface SourceIntentStorage {
  read(key: string): Promise<SourceIntent | undefined>;
  put(intent: SourceIntent): Promise<void>;
  remove(intent: SourceIntent): Promise<void>;
}
type Snapshot = {
  pending: SourceIntent | null;
  /**
   * Whether the last attempt ended without a confirmed result, or a stored
   * request was recovered at startup. A request that is simply in flight is
   * not unconfirmed: the reading position writes continuously while scrolling,
   * and surfacing those in-flight windows made the reader's notice and layout
   * flash several times per second.
   */
  unconfirmed: boolean;
  ready: boolean;
  busy: boolean;
  error: string | null;
  generation: number;
  locked: boolean;
  lastJob: { commandId: string; jobId: string } | null;
};
const describe = (e: unknown) => (e instanceof Error ? e.message : 'Operation failed / 操作未完成');
const kinds = new Set([
  'source_job_retry',
  'sources_import',
  'sources_import_webpage',
  'sources_update',
  'sources_set_trashed',
  'annotations_create',
  'annotations_update',
  'annotations_set_trashed',
  'reading_set',
  'objects_set_archived',
  'objects_set_search_enabled',
  'objects_set_inbox_state',
  'tags_assign',
  'tags_remove',
]);
export function intentId(intent: SourceIntent) {
  const payload = 'payload' in intent.command ? intent.command.payload : null;
  return payload && typeof payload === 'object' && 'command_id' in payload
    ? payload.command_id
    : undefined;
}
export function validateIntent(intent: SourceIntent, key: string, identity: WorkspaceIdentity) {
  if (
    !intent ||
    intent.key !== key ||
    intent.identity?.local_instance_id !== identity.local_instance_id ||
    intent.identity?.workspace_id !== identity.workspace_id ||
    !kinds.has(intent.command?.kind) ||
    typeof intentId(intent) !== 'string' ||
    !['source_mutation', 'metadata_commit', 'job_accepted'].includes(intent.expected)
  )
    throw new Error(
      'Pending request is invalid; preserved for recovery / 待确认请求不匹配，已保留供恢复',
    );
  const expected =
    intent.command.kind === 'source_job_retry' || intent.command.kind.startsWith('sources_import')
      ? 'job_accepted'
      : /^(objects_|tags_)/.test(intent.command.kind)
        ? 'metadata_commit'
        : 'source_mutation';
  if (intent.expected !== expected)
    throw new Error('Pending response type does not match command / 待确认命令与回执类型不匹配');
}
/** Shared per scope across React remounts and duplicate panes. */
export class SourceActions {
  private snapshot: Snapshot = {
    pending: null,
    unconfirmed: false,
    ready: false,
    busy: false,
    error: null,
    generation: 0,
    locked: true,
    lastJob: null,
  };
  private listeners = new Set<() => void>();
  private loading: Promise<void> | null = null;
  private key: string;
  private identity: WorkspaceIdentity;
  private storage: SourceIntentStorage;
  private send: (intent: SourceIntent) => Promise<unknown>;
  constructor(
    key: string,
    identity: WorkspaceIdentity,
    storage: SourceIntentStorage,
    send: (intent: SourceIntent) => Promise<unknown>,
  ) {
    this.key = key;
    this.identity = identity;
    this.storage = storage;
    this.send = send;
  }
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private update(patch: Partial<Snapshot>) {
    const state = { ...this.snapshot, ...patch };
    state.locked = !state.ready || state.busy || !!state.pending;
    this.snapshot = state;
    this.listeners.forEach((listener) => listener());
  }
  load = async () => {
    if (this.snapshot.ready) return;
    if (this.loading) return this.loading;
    this.loading = this.storage
      .read(this.key)
      .then((intent) => {
        if (intent) validateIntent(intent, this.key, this.identity);
        this.update({
          pending: intent ?? null,
          unconfirmed: !!intent,
          ready: true,
          error: null,
        });
      })
      .catch((e) => {
        this.update({ error: describe(e) });
      })
      .finally(() => {
        this.loading = null;
      });
    return this.loading;
  };
  private async dispatch(intent: SourceIntent) {
    try {
      const result = await this.send(structuredClone(intent));
      // Keep the accepted task identity even when removing its local receipt fails.
      // A retry still replays the original command and resolves to the same task.
      if (
        intent.expected === 'job_accepted' &&
        result &&
        typeof result === 'object' &&
        'job_id' in result &&
        typeof result.job_id === 'string'
      )
        this.update({ lastJob: { commandId: String(intentId(intent)), jobId: result.job_id } });
      await this.storage.remove(intent);
      this.update({
        pending: null,
        unconfirmed: false,
        error: null,
        generation: this.snapshot.generation + 1,
      });
      return true;
    } catch (e) {
      const code = (e as { details?: { code?: string } })?.details?.code;
      if (
        code &&
        [
          'invalid_input',
          'not_found',
          'metadata_conflict',
          'object_trashed',
          'input_unavailable',
        ].includes(code)
      ) {
        await this.storage.remove(intent);
        this.update({
          pending: null,
          unconfirmed: false,
          generation: this.snapshot.generation + 1,
        });
        this.update({ error: describe(e) });
        return false;
      }
      this.update({ error: describe(e), unconfirmed: true });
      return false;
    }
  }
  execute = async (command: DesktopCommand, expected: DesktopData['kind']) => {
    if (this.snapshot.locked) return false;
    this.update({ busy: true, error: null, unconfirmed: false });
    const intent = structuredClone({ key: this.key, identity: this.identity, command, expected });
    try {
      validateIntent(intent, this.key, this.identity);
      await this.storage.put(intent);
      this.update({ pending: intent });
      return await this.dispatch(intent);
    } catch (e) {
      this.update({ error: describe(e) });
      return false;
    } finally {
      this.update({ busy: false });
    }
  };
  retry = async () => {
    if (!this.snapshot.ready) {
      await this.load();
      return;
    }
    const intent = this.snapshot.pending;
    if (!intent || this.snapshot.busy) return;
    this.update({ busy: true });
    try {
      await this.dispatch(intent);
    } catch (e) {
      this.update({ error: describe(e) });
    } finally {
      this.update({ busy: false });
    }
  };
}

import type {
  DesktopCommand,
  MetadataCommit,
  ObjectKind,
  TrashCommit,
  TrashItem,
  TrashPreview,
  TrashSelection,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { actionErrorCode } from './page-actions.ts';

export type TrashDeleteCommand = Extract<DesktopCommand, { kind: 'trash_delete' }>;
export type TrashRestoreCommand = Extract<DesktopCommand, { kind: 'objects_set_trashed' }>;
type Rejection = { error: string; code: string };
type IntentBase = { schema: 1; key: string; identity: WorkspaceIdentity; operation_id: string };
export type DeleteTrashIntent = IntentBase & {
  kind: 'delete';
  preview: TrashPreview;
  command: TrashDeleteCommand;
  commit: TrashCommit | null;
  rejection: Rejection | null;
};
export type RestoreTrashItem = {
  item: TrashItem;
  command: TrashRestoreCommand;
  status: 'ready' | 'pending' | 'done' | 'failed';
  error: string | null;
  code: string | null;
};
export type RestoreTrashIntent = IntentBase & { kind: 'restore'; items: RestoreTrashItem[] };
export type TrashIntent = DeleteTrashIntent | RestoreTrashIntent;
export interface TrashIntentStorage {
  read(key: string): Promise<TrashIntent | undefined>;
  write(intent: TrashIntent): Promise<void>;
  remove(key: string): Promise<void>;
}
export type TrashActionOutcome = {
  operation_id: string;
  kind: 'delete' | 'restore';
  total_count: number;
  items: {
    object_id: string;
    kind: ObjectKind;
    title: string;
    status: 'done' | 'failed';
    error: string | null;
    code: string | null;
  }[];
};
export type TrashActionSnapshot = {
  busy: boolean;
  pending: TrashIntent | null;
  error: string | null;
  code: string | null;
  generation: number;
  result: TrashActionOutcome | null;
};
export type TrashTransport = {
  delete: (command: TrashDeleteCommand) => Promise<TrashCommit>;
  restore: (command: TrashRestoreCommand) => Promise<MetadataCommit>;
  finalizeDelete?: (commit: TrashCommit) => Promise<void>;
};
export const trashKey = (identity: WorkspaceIdentity) =>
  `${identity.local_instance_id}/${identity.workspace_id}/trash-lifecycle`;
const terminal = new Set([
  'query_expired',
  'metadata_conflict',
  'invalid_input',
  'object_trashed',
  'not_found',
  'command_conflict',
]);
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const kinds = new Set(['note', 'source', 'annotation', 'collection']);
const nonempty = (value: unknown): value is string => typeof value === 'string' && !!value.trim();
const unique = (values: readonly string[]) => new Set(values).size === values.length;

function validItem(item: TrashItem): boolean {
  return (
    !!item &&
    nonempty(item.object_id) &&
    kinds.has(item.kind) &&
    typeof item.title === 'string' &&
    typeof item.preview === 'string' &&
    Number.isSafeInteger(item.row_version) &&
    item.row_version > 0 &&
    (item.updated_at === null || typeof item.updated_at === 'string') &&
    (item.parent_source_id === null || nonempty(item.parent_source_id)) &&
    typeof item.source_trashed === 'boolean'
  );
}
function validSelection(selection: TrashSelection): boolean {
  return (
    !!selection &&
    (selection.kind === 'all' ||
      (selection.kind === 'objects' &&
        Array.isArray(selection.object_ids) &&
        selection.object_ids.length > 0 &&
        selection.object_ids.every(nonempty) &&
        unique(selection.object_ids)))
  );
}
function sameSelection(left: TrashSelection, right: TrashSelection): boolean {
  if (!validSelection(left) || !validSelection(right) || left.kind !== right.kind) return false;
  if (left.kind === 'all' || right.kind === 'all') return true;
  const ids = new Set(left.object_ids);
  return ids.size === right.object_ids.length && right.object_ids.every((id) => ids.has(id));
}
function validPreview(preview: TrashPreview): boolean {
  return (
    !!preview &&
    validSelection(preview.selection) &&
    nonempty(preview.snapshot_token) &&
    Array.isArray(preview.items) &&
    preview.items.length > 0 &&
    preview.items.every(validItem) &&
    unique(preview.items.map((item) => item.object_id)) &&
    Number.isSafeInteger(preview.selected_count) &&
    preview.selected_count > 0 &&
    Number.isSafeInteger(preview.dependent_count) &&
    preview.dependent_count >= 0 &&
    preview.total_count === preview.items.length &&
    preview.total_count === preview.selected_count + preview.dependent_count &&
    (preview.selection.kind === 'all' ||
      (preview.selection.object_ids.length === preview.selected_count &&
        preview.selection.object_ids.every((id) =>
          preview.items.some((item) => item.object_id === id),
        )))
  );
}
function validCommit(commit: TrashCommit, intent: DeleteTrashIntent): boolean {
  if (
    !commit ||
    commit.receipt?.command_id !== intent.command.payload.command_id ||
    !Array.isArray(commit.deleted_objects) ||
    commit.deleted_objects.length !== intent.preview.items.length
  )
    return false;
  const expected = new Map(intent.preview.items.map((item) => [item.object_id, item.kind]));
  return (
    unique(commit.deleted_objects.map((item) => item.object_id)) &&
    commit.deleted_objects.every((item) => expected.get(item.object_id) === item.kind)
  );
}
function validIntent(intent: TrashIntent, identity: WorkspaceIdentity): boolean {
  if (
    intent.schema !== 1 ||
    intent.key !== trashKey(identity) ||
    !intent.identity ||
    intent.identity.local_instance_id !== identity.local_instance_id ||
    intent.identity.workspace_id !== identity.workspace_id ||
    !nonempty(intent.operation_id)
  )
    return false;
  if (intent.kind === 'delete')
    return (
      validPreview(intent.preview) &&
      intent.command?.kind === 'trash_delete' &&
      intent.command.payload?.command_id === intent.operation_id &&
      intent.command.payload.request?.snapshot_token === intent.preview.snapshot_token &&
      sameSelection(intent.command.payload.request.selection, intent.preview.selection) &&
      (intent.commit === null || validCommit(intent.commit, intent)) &&
      (intent.rejection === null ||
        (nonempty(intent.rejection?.error) && terminal.has(intent.rejection?.code))) &&
      !(intent.commit && intent.rejection)
    );
  return (
    intent.kind === 'restore' &&
    Array.isArray(intent.items) &&
    intent.items.length > 0 &&
    unique(intent.items.map((item) => item.item?.object_id)) &&
    unique(intent.items.map((item) => item.command?.payload?.command_id)) &&
    intent.items.every(
      ({ item, command, status }) =>
        validItem(item) &&
        command?.kind === 'objects_set_trashed' &&
        nonempty(command.payload?.command_id) &&
        command.payload.request?.object_id === item.object_id &&
        command.payload.request.base_row_version === item.row_version &&
        command.payload.request.value === false &&
        ['ready', 'pending', 'done', 'failed'].includes(status),
    )
  );
}
export function trashOutcome(intent: TrashIntent): TrashActionOutcome | null {
  const items: TrashActionOutcome['items'] =
    intent.kind === 'delete'
      ? intent.commit
        ? intent.preview.items.map((item) => ({
            object_id: item.object_id,
            kind: item.kind,
            title: item.title,
            status: 'done',
            error: null,
            code: null,
          }))
        : []
      : intent.items.flatMap(({ item, status, error, code }) =>
          status === 'done' || status === 'failed'
            ? [
                {
                  object_id: item.object_id,
                  kind: item.kind,
                  title: item.title,
                  status,
                  error,
                  code,
                },
              ]
            : [],
        );
  return items.length
    ? {
        operation_id: intent.operation_id,
        kind: intent.kind,
        total_count: intent.kind === 'delete' ? intent.preview.total_count : intent.items.length,
        items,
      }
    : null;
}

/** Persist confirmed scope and exact commands before any mutation; never enlarge a retry. */
export class TrashActions {
  private state: TrashActionSnapshot = {
    busy: false,
    pending: null,
    error: null,
    code: null,
    generation: 0,
    result: null,
  };
  private listeners = new Set<() => void>();
  private identity: WorkspaceIdentity;
  private storage: TrashIntentStorage;
  private transport: TrashTransport;
  private constructor(
    identity: WorkspaceIdentity,
    storage: TrashIntentStorage,
    transport: TrashTransport,
  ) {
    this.identity = structuredClone(identity);
    this.storage = storage;
    this.transport = transport;
  }
  static async open(
    identity: WorkspaceIdentity,
    storage: TrashIntentStorage,
    transport: TrashTransport,
  ) {
    const controller = new TrashActions(identity, storage, transport);
    const intent = await storage.read(trashKey(identity));
    if (intent && !validIntent(intent, identity))
      throw new Error('Trash operation record is invalid / 回收站操作记录校验失败，未发送操作');
    controller.state = {
      ...controller.state,
      pending: intent ?? null,
      result: intent ? trashOutcome(intent) : null,
    };
    return controller;
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private emit(patch: Partial<TrashActionSnapshot>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  private async persist(intent: TrashIntent) {
    // Keep the acknowledged state in memory if its disk update fails. A restart
    // sees the older exact command and confirms its immutable core receipt.
    this.emit({ pending: structuredClone(intent) });
    await this.storage.write(structuredClone(intent));
  }
  private publish(intent: TrashIntent) {
    this.emit({ result: trashOutcome(intent), generation: this.state.generation + 1 });
  }
  async delete(preview: TrashPreview): Promise<TrashActionOutcome | null> {
    if (this.state.busy || this.state.pending) return null;
    if (!validPreview(preview)) throw new Error('Deletion preview is invalid / 删除范围校验失败');
    const operation_id = crypto.randomUUID();
    this.emit({
      result: null,
      pending: {
        schema: 1,
        key: trashKey(this.identity),
        identity: structuredClone(this.identity),
        operation_id,
        kind: 'delete',
        preview: structuredClone(preview),
        command: {
          kind: 'trash_delete',
          payload: {
            command_id: operation_id,
            request: {
              selection: structuredClone(preview.selection),
              snapshot_token: preview.snapshot_token,
            },
          },
        },
        commit: null,
        rejection: null,
      },
    });
    return this.retry();
  }
  async restore(items: TrashItem[]): Promise<TrashActionOutcome | null> {
    if (this.state.busy || this.state.pending) return null;
    if (!items.length || !items.every(validItem) || !unique(items.map((item) => item.object_id)))
      throw new Error('Restoration selection is invalid / 恢复范围校验失败');
    const selected = new Set(items.map((item) => item.object_id));
    if (
      items.some(
        (item) =>
          item.kind === 'annotation' &&
          item.source_trashed &&
          (!item.parent_source_id || !selected.has(item.parent_source_id)),
      )
    )
      throw new Error('Restore the source first / 请先恢复摘录所属的资料');
    this.emit({
      result: null,
      pending: {
        schema: 1,
        key: trashKey(this.identity),
        identity: structuredClone(this.identity),
        operation_id: crypto.randomUUID(),
        kind: 'restore',
        items: [...items]
          .sort((left, right) => Number(right.kind === 'source') - Number(left.kind === 'source'))
          .map((item) => ({
            item: structuredClone(item),
            command: {
              kind: 'objects_set_trashed',
              payload: {
                command_id: crypto.randomUUID(),
                request: {
                  object_id: item.object_id,
                  base_row_version: item.row_version,
                  value: false,
                },
              },
            },
            status: 'ready',
            error: null,
            code: null,
          })),
      },
    });
    return this.retry();
  }
  async retry(): Promise<TrashActionOutcome | null> {
    if (this.state.busy || !this.state.pending) return null;
    this.emit({ busy: true, error: null, code: null });
    const intent = structuredClone(this.state.pending);
    try {
      await this.persist(intent);
      if (intent.kind === 'delete') {
        await this.deletePending(intent);
        // Keep the durable core receipt until local drafts are removed too.
        // A failed cleanup retries locally, never inventing a new deletion.
        if (intent.commit) await this.transport.finalizeDelete?.(structuredClone(intent.commit));
      } else await this.restorePending(intent);
      await this.storage.remove(intent.key);
      this.emit({
        pending: null,
        result: trashOutcome(intent),
        error: intent.kind === 'delete' ? (intent.rejection?.error ?? null) : null,
        code: intent.kind === 'delete' ? (intent.rejection?.code ?? null) : null,
      });
    } catch (error) {
      this.emit({ error: errorText(error), code: actionErrorCode(error) ?? null });
    } finally {
      this.emit({ busy: false });
    }
    return this.state.result;
  }
  private async deletePending(intent: DeleteTrashIntent) {
    if (intent.commit || intent.rejection) return;
    try {
      const commit = await this.transport.delete(structuredClone(intent.command));
      if (!validCommit(commit, intent))
        throw new Error('Deletion receipt did not match / 删除回执不匹配，请重试原操作确认');
      intent.commit = structuredClone(commit);
      this.publish(intent);
    } catch (error) {
      const code = actionErrorCode(error);
      if (!code || !terminal.has(code)) throw error;
      intent.rejection = { error: errorText(error), code };
    }
    await this.persist(intent);
  }
  private async restorePending(intent: RestoreTrashIntent) {
    for (const entry of intent.items) {
      if (entry.status === 'done' || entry.status === 'failed') continue;
      const parent = intent.items.find(
        (other) => other.item.object_id === entry.item.parent_source_id,
      );
      if (entry.item.source_trashed && parent?.status === 'failed') {
        entry.status = 'failed';
        entry.error =
          'Source restoration failed; restore it first / 所属资料恢复失败，请先恢复资料';
        entry.code = 'object_trashed';
        this.publish(intent);
        await this.persist(intent);
        continue;
      }
      entry.status = 'pending';
      entry.error = null;
      entry.code = null;
      await this.persist(intent);
      try {
        const commit = await this.transport.restore(structuredClone(entry.command));
        if (
          commit.receipt?.command_id !== entry.command.payload.command_id ||
          !commit.objects.some((item) => item.id === entry.item.object_id && !item.trashed)
        )
          throw new Error('Restoration receipt did not match / 恢复回执不匹配，请重试原操作确认');
        entry.status = 'done';
      } catch (error) {
        const code = actionErrorCode(error) ?? null;
        entry.error = errorText(error);
        entry.code = code;
        if (!code || !terminal.has(code)) {
          await this.persist(intent);
          throw error;
        }
        entry.status = 'failed';
      }
      this.publish(intent);
      await this.persist(intent);
    }
  }
  dismissResult() {
    if (!this.state.busy && !this.state.pending)
      this.emit({ result: null, error: null, code: null });
  }
}

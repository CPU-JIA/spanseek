import type { InboxState, ObjectMetadata, WorkspaceIdentity } from '@pkms/contracts';
import type { MetadataController, MetadataMutation } from './metadata-controller.ts';
import { activeLanguage } from '../language.ts';

/** The kernel accepts 1–100 targets per metadata mutation. */
export const BULK_LIMIT = 100;

export type BulkAction = { kind: 'tag'; name: string } | { kind: 'inbox'; target: InboxState };
export type BulkItem = {
  id: string;
  title: string;
  command?: MetadataMutation;
  status: 'ready' | 'pending' | 'done' | 'failed';
  error?: string;
};
export type BulkRecord = { schema: 1; key: string; action: BulkAction; items: BulkItem[] };
export interface BulkStorage {
  read(key: string): Promise<BulkRecord | undefined>;
  write(record: BulkRecord): Promise<void>;
  remove(key: string): Promise<void>;
}
const keyFor = (identity: WorkspaceIdentity) =>
  `${identity.local_instance_id}/${identity.workspace_id}/bulk`;
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
function validItem(item: BulkItem) {
  if (
    !item ||
    typeof item.id !== 'string' ||
    !item.id ||
    typeof item.title !== 'string' ||
    !['ready', 'pending', 'done', 'failed'].includes(item.status)
  )
    return false;
  if (!item.command) return item.status === 'ready' || item.status === 'failed';
  const command = item.command;
  if (!command.payload?.command_id) return false;
  if (command.kind === 'objects_set_inbox_state')
    return command.payload.request?.object_id === item.id;
  return (
    command.kind === 'tags_assign' &&
    command.payload.request?.targets?.length === 1 &&
    command.payload.request.targets[0].object_id === item.id
  );
}
function build(action: BulkAction, metadata: ObjectMetadata): MetadataMutation {
  const command_id = crypto.randomUUID();
  const target = { object_id: metadata.id, base_row_version: metadata.row_version };
  return action.kind === 'tag'
    ? {
        kind: 'tags_assign',
        payload: {
          command_id,
          request: { targets: [target], tag: { kind: 'name', name: action.name } },
        },
      }
    : {
        kind: 'objects_set_inbox_state',
        payload: { command_id, request: { ...target, target: action.target } },
      };
}

/** Persist the exact batch and each exact command before using the shared note queue. */
export class BulkActions {
  private state: { busy: boolean; record: BulkRecord | null; error: string | null } = {
    busy: false,
    record: null,
    error: null,
  };
  private listeners = new Set<() => void>();
  private key: string;
  private storage: BulkStorage;
  private metadata: (id: string) => Promise<MetadataController>;
  private constructor(
    key: string,
    storage: BulkStorage,
    metadata: (id: string) => Promise<MetadataController>,
  ) {
    this.key = key;
    this.storage = storage;
    this.metadata = metadata;
  }
  static async open(
    identity: WorkspaceIdentity,
    storage: BulkStorage,
    metadata: (id: string) => Promise<MetadataController>,
  ) {
    const controller = new BulkActions(keyFor(identity), storage, metadata);
    const record = await storage.read(controller.key);
    if (
      record &&
      (record.schema !== 1 ||
        record.key !== controller.key ||
        !record.action ||
        (record.action.kind !== 'tag' && record.action.kind !== 'inbox') ||
        !Array.isArray(record.items) ||
        !record.items.every(validItem) ||
        record.items.length > 100 ||
        new Set(record.items.map((item) => item.id)).size !== record.items.length)
    )
      throw new Error('Invalid organization record / 整理记录校验失败');
    controller.state.record = record ?? null;
    return controller;
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private emit(patch: Partial<typeof this.state>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  private async save(record: BulkRecord) {
    await this.storage.write(structuredClone(record));
    this.emit({ record: structuredClone(record) });
  }
  async start(notes: { id: string; title: string }[], action: BulkAction) {
    if (this.state.busy || this.state.record || !notes.length) return;
    if (notes.length > BULK_LIMIT) {
      // The selection UI already caps the batch, so reaching this means a caller
      // bypassed it. Report the limit instead of silently doing nothing.
      this.emit({
        error:
          activeLanguage() === 'en'
            ? `Choose 1–${BULK_LIMIT} notes per batch.`
            : `每批最多选择 ${BULK_LIMIT} 篇笔记。`,
      });
      return;
    }
    this.emit({ busy: true, error: null });
    try {
      const unique = [...new Map(notes.map((note) => [note.id, note])).values()];
      await this.save({
        schema: 1,
        key: this.key,
        action,
        items: unique.map((note) => ({ ...note, status: 'ready' })),
      });
    } catch (error) {
      this.emit({ error: errorText(error) });
    } finally {
      this.emit({ busy: false });
    }
    if (this.state.record) await this.resume();
  }
  async resume() {
    if (this.state.busy || !this.state.record) return;
    this.emit({ busy: true, error: null });
    const record = structuredClone(this.state.record);
    try {
      for (const item of record.items) {
        if (item.status === 'done' || item.status === 'failed') continue;
        const controller = await this.metadata(item.id);
        const pending = controller.getPendingCommand();
        if (
          controller.getSnapshot().busy ||
          (pending && pending.payload.command_id !== item.command?.payload.command_id)
        ) {
          item.error =
            'Another change needs confirmation in this note / 请先在笔记中确认另一项整理操作';
          await this.save(record);
          continue;
        }
        const before = controller.getSnapshot().generation;
        const success = await controller.mutate(
          pending
            ? undefined
            : async (current) => {
                if (!item.command) {
                  item.command = build(record.action, current);
                  item.status = 'pending';
                }
                await this.save(record);
                return structuredClone(item.command);
              },
        );
        const snapshot = controller.getSnapshot();
        const committed = success || snapshot.generation > before;
        item.status = committed ? 'done' : snapshot.pending ? 'pending' : 'failed';
        item.error = committed
          ? undefined
          : (snapshot.error ?? 'Change was not applied / 操作未应用');
        await this.save(record);
      }
    } catch (error) {
      this.emit({ error: errorText(error) });
    } finally {
      this.emit({ busy: false });
    }
  }
  async dismiss() {
    if (
      this.state.busy ||
      this.state.record?.items.some((item) => item.status === 'pending' || item.status === 'ready')
    )
      return;
    try {
      await this.storage.remove(this.key);
      this.emit({ record: null, error: null });
    } catch (error) {
      this.emit({ error: errorText(error) });
    }
  }
}

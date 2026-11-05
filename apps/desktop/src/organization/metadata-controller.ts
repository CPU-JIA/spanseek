import type { DesktopCommand, ObjectMetadata, WorkspaceIdentity } from '@pkms/contracts';
import type { DraftController } from '../editor/draft-controller.ts';
import { draftKey } from '../editor/drafts.ts';
import type { IntentStorage, MetadataIntent } from './intent-storage.ts';

export type MetadataMutation = Extract<
  DesktopCommand,
  {
    kind: 'tags_assign' | 'tags_remove' | 'objects_set_inbox_state';
  }
>;
type Snapshot = {
  metadata: ObjectMetadata | null;
  busy: boolean;
  pending: boolean;
  error: string | null;
  generation: number;
};
type Editor = Pick<DraftController, 'flush' | 'beginAction'>;
const errorMessage = (error: unknown) =>
  error instanceof Error ? error.message : '整理操作失败，请重试。';
const errorCode = (error: unknown) =>
  error && typeof error === 'object' && 'details' in error
    ? (error.details as { code?: string } | null)?.code
    : undefined;

/** One durable mutation queue shared by both panes of the same note. */
export class MetadataController {
  private snapshot: Snapshot = {
    metadata: null,
    busy: false,
    pending: false,
    error: null,
    generation: 0,
  };
  private listeners = new Set<() => void>();
  private pending: MetadataIntent | undefined;
  private sequence = 0;
  private identity: WorkspaceIdentity;
  private objectId: string;
  private editor: Editor;
  private storage: IntentStorage;
  private read: () => Promise<ObjectMetadata>;
  private send: (command: MetadataMutation) => Promise<unknown>;
  private constructor(
    identity: WorkspaceIdentity,
    objectId: string,
    editor: Editor,
    storage: IntentStorage,
    read: () => Promise<ObjectMetadata>,
    send: (command: MetadataMutation) => Promise<unknown>,
  ) {
    this.identity = identity;
    this.objectId = objectId;
    this.editor = editor;
    this.storage = storage;
    this.read = read;
    this.send = send;
  }
  static async open(
    identity: WorkspaceIdentity,
    objectId: string,
    editor: Editor,
    storage: IntentStorage,
    read: () => Promise<ObjectMetadata>,
    send: (command: MetadataMutation) => Promise<unknown>,
  ) {
    const result = new MetadataController(identity, objectId, editor, storage, read, send);
    const key = draftKey(identity, objectId);
    const stored = await storage.read(key);
    if (stored && (stored.key !== key || draftKey(stored.identity, stored.objectId) !== key))
      throw new Error('整理操作身份校验失败。');
    result.pending = stored;
    result.emit({ pending: !!stored });
    await result.refresh();
    return result;
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  getSnapshot = () => this.snapshot;
  getPendingCommand = () => (this.pending ? structuredClone(this.pending.command) : undefined);
  private emit(patch: Partial<Snapshot>) {
    this.snapshot = { ...this.snapshot, ...patch };
    for (const listener of this.listeners) listener();
  }
  async refresh() {
    const sequence = ++this.sequence;
    try {
      const metadata = await this.read();
      if (sequence === this.sequence) this.emit({ metadata });
      return metadata;
    } catch (error) {
      if (sequence === this.sequence) this.emit({ error: errorMessage(error) });
      throw error;
    }
  }
  async mutate(
    build?: (metadata: ObjectMetadata) => MetadataMutation | Promise<MetadataMutation>,
  ): Promise<boolean> {
    if (this.snapshot.busy) return false;
    // A new action must never silently replay a different, unresolved intent.
    if (this.pending && build) return false;
    this.emit({ busy: true, error: null });
    let finish: (() => void) | undefined;
    let sent = false;
    try {
      finish = this.editor.beginAction();
      if (!this.pending) {
        if (!build) return false;
        await this.editor.flush();
        const command = await build(await this.refresh());
        const intent = {
          key: draftKey(this.identity, this.objectId),
          identity: this.identity,
          objectId: this.objectId,
          command,
        };
        await this.storage.write(intent);
        this.pending = intent;
        this.emit({ pending: true });
      }
      sent = true;
      await this.send(this.pending.command);
      await this.storage.remove(this.pending.key);
      this.pending = undefined;
      this.emit({ pending: false, generation: this.snapshot.generation + 1 });
      // A replay returns its original commit snapshot. Read the latest state.
      await this.refresh();
      return true;
    } catch (error) {
      if (
        sent &&
        ['metadata_conflict', 'invalid_input', 'object_trashed', 'not_found'].includes(
          errorCode(error) ?? '',
        ) &&
        this.pending
      ) {
        try {
          await this.storage.remove(this.pending.key);
          this.pending = undefined;
          this.emit({ pending: false });
          await this.refresh();
        } catch (refreshError) {
          this.emit({ error: `${errorMessage(error)}；${errorMessage(refreshError)}` });
          return false;
        }
      }
      this.emit({
        error:
          errorCode(error) === 'metadata_conflict'
            ? '标签或整理状态已有更新，请检查当前状态后再操作。'
            : errorMessage(error),
      });
      return false;
    } finally {
      finish?.();
      this.emit({ busy: false });
    }
  }
}

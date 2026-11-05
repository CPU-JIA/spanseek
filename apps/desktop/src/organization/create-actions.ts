import type { DesktopCommand, NoteCommit, WorkspaceIdentity } from '@pkms/contracts';
import { actionErrorCode } from './page-actions.ts';

export type CreateCommand = Extract<
  DesktopCommand,
  { kind: 'notes_create' | 'sources_create_note' }
>;
export type CreateIntent = {
  schema: 1;
  key: string;
  identity: WorkspaceIdentity;
  command: CreateCommand;
  result: NoteCommit | null;
};
export interface CreateStorage {
  read(key: string): Promise<CreateIntent | undefined>;
  write(intent: CreateIntent): Promise<void>;
  remove(key: string): Promise<void>;
}
export const createKey = (identity: WorkspaceIdentity) =>
  `${identity.local_instance_id}/${identity.workspace_id}/note-create`;
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
function validReceipt(command: CreateCommand, result: NoteCommit) {
  return (
    !!result?.note?.id &&
    !!result.note.revision_id &&
    result.receipt?.command_id === command.payload.command_id &&
    result.receipt.object_id === result.note.id &&
    result.receipt.revision_id === result.note.revision_id
  );
}

/** A create remains durable until its confirmed note has been opened successfully. */
export class CreateActions {
  private state: {
    busy: boolean;
    pending: CreateIntent | null;
    result: NoteCommit | null;
    error: string | null;
    code: string | null;
    generation: number;
  } = { busy: false, pending: null, result: null, error: null, code: null, generation: 0 };
  private listeners = new Set<() => void>();
  private identity: WorkspaceIdentity;
  private storage: CreateStorage;
  private send: (command: CreateCommand) => Promise<NoteCommit>;
  private constructor(
    identity: WorkspaceIdentity,
    storage: CreateStorage,
    send: (command: CreateCommand) => Promise<NoteCommit>,
  ) {
    this.identity = structuredClone(identity);
    this.storage = storage;
    this.send = send;
  }
  static async open(
    identity: WorkspaceIdentity,
    storage: CreateStorage,
    send: (command: CreateCommand) => Promise<NoteCommit>,
  ) {
    const controller = new CreateActions(identity, storage, send);
    const stored = await storage.read(createKey(identity));
    if (
      stored &&
      (stored.schema !== 1 ||
        stored.key !== createKey(identity) ||
        !stored.identity ||
        createKey(stored.identity) !== stored.key ||
        !['notes_create', 'sources_create_note'].includes(stored.command?.kind) ||
        !stored.command.payload?.command_id ||
        !(stored.command.kind === 'notes_create'
          ? stored.command.payload.note
          : stored.command.payload.request) ||
        (stored.result && !validReceipt(stored.command, stored.result)))
    )
      throw new Error('Invalid pending note creation / 新建笔记记录校验失败');
    controller.state = {
      ...controller.state,
      pending: stored ?? null,
      result: stored?.result ?? null,
    };
    return controller;
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  getSnapshot = () => this.state;
  private emit(patch: Partial<typeof this.state>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  async create(command?: CreateCommand): Promise<NoteCommit | null> {
    if (this.state.busy) return null;
    // Even a repeated New command resolves the outstanding note; it never replaces it.
    if (!this.state.pending && !command) return null;
    if (this.state.result) return structuredClone(this.state.result);
    this.emit({ busy: true, error: null, code: null });
    let sent = false;
    try {
      if (!this.state.pending)
        this.emit({
          pending: {
            schema: 1,
            key: createKey(this.identity),
            identity: this.identity,
            command: structuredClone(command!),
            result: null,
          },
        });
      const intent = this.state.pending!;
      await this.storage.write(structuredClone(intent));
      sent = true;
      const result = await this.send(structuredClone(intent.command));
      if (!validReceipt(intent.command, result))
        throw new Error('Note creation receipt does not match / 新建笔记回执不匹配，请确认原操作');
      const confirmed = { ...intent, result: structuredClone(result) };
      // Keep the original intent if persisting the success fails. A replay recovers it.
      await this.storage.write(confirmed);
      this.emit({ pending: confirmed, result, generation: this.state.generation + 1 });
      return structuredClone(result);
    } catch (error) {
      const code = actionErrorCode(error) ?? null;
      const collectionRejected =
        this.state.pending?.command.kind === 'notes_create' &&
        !!this.state.pending?.command.payload.note.collection_id &&
        ['metadata_conflict', 'object_trashed', 'not_found'].includes(code ?? '');
      const sourceRejected =
        this.state.pending?.command.kind === 'sources_create_note' &&
        ['invalid_document', 'object_trashed', 'not_found'].includes(code ?? '');
      if (
        sent &&
        (code === 'invalid_input' || collectionRejected || sourceRejected) &&
        this.state.pending
      ) {
        try {
          await this.storage.remove(this.state.pending.key);
          this.emit({ pending: null });
        } catch (storageError) {
          this.emit({ error: `${errorText(error)}; ${errorText(storageError)}`, code });
          return null;
        }
      }
      this.emit({ error: errorText(error), code });
      return null;
    } finally {
      this.emit({ busy: false });
    }
  }
  async acknowledgeOpened(noteId: string): Promise<boolean> {
    if (this.state.busy || !this.state.pending || this.state.result?.note.id !== noteId)
      return false;
    this.emit({ busy: true, error: null });
    try {
      await this.storage.remove(this.state.pending.key);
      this.emit({ pending: null, result: null });
      return true;
    } catch (error) {
      this.emit({ error: errorText(error) });
      return false;
    } finally {
      this.emit({ busy: false });
    }
  }
}

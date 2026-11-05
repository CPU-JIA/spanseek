import type { Note, NoteCommit, NoteDocument, SaveNote, WorkspaceIdentity } from '@pkms/contracts';
import { draftKey, type DraftRecord, type DraftStorage, type HistoryCommand } from './drafts.ts';
import { EDITOR_DOCUMENT_SCHEMA } from './document-schema.ts';
import { mergeCommittedBindings } from './committed-bindings.ts';
import { duplicateTitle } from './duplicate-title.ts';

export interface DraftSnapshot {
  record: DraftRecord;
  status: 'saved' | 'dirty' | 'local' | 'saving' | 'error' | 'conflict';
  error: string | null;
  historyBusy: boolean;
}
type Save = (commandId: string, request: SaveNote) => Promise<NoteCommit>;
function conflict(error: unknown): boolean {
  return (
    !!error &&
    typeof error === 'object' &&
    'details' in error &&
    (error.details as { code?: string } | null)?.code === 'revision_conflict'
  );
}
function rejectedInput(error: unknown): boolean {
  return (
    !!error &&
    typeof error === 'object' &&
    'details' in error &&
    ['invalid_input', 'invalid_document'].includes(
      (error.details as { code?: string } | null)?.code ?? '',
    )
  );
}

interface LocalWrite {
  record: DraftRecord;
  promise: Promise<void>;
  resolve: () => void;
  reject: (error: unknown) => void;
}
function localWrite(record: DraftRecord): LocalWrite {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<void>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { record, promise, resolve, reject };
}

/** Exactly one instance per workspace identity + note; React panes subscribe to it. */
export class DraftController {
  private snapshot: DraftSnapshot;
  private readonly listeners = new Set<() => void>();
  /** At most one record waits here; it always holds the newest edits. */
  private queued: LocalWrite | null = null;
  private writing: LocalWrite | null = null;
  /** Settles when the in-flight local write finishes, including its failure. */
  private writingRun: Promise<void> | null = null;
  /** True inside a 650 ms window, where later edits merge instead of starting a write. */
  private merging = false;
  private running: Promise<void> | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private composing = new Set<string>();
  private stopped = false;
  private frozen = false;
  private actions = 0;
  private adopting = false;
  private storage: DraftStorage;
  private save: Save;

  private constructor(record: DraftRecord, storage: DraftStorage, save: Save, stale: boolean) {
    this.storage = storage;
    this.save = save;
    this.snapshot = {
      record,
      historyBusy: false,
      status: record.historyIntent
        ? 'error'
        : stale
          ? 'conflict'
          : record.pending || record.generation !== record.acknowledged
            ? 'local'
            : 'saved',
      error: record.historyIntent
        ? '版本操作尚未确认，草稿与原请求已保留。'
        : stale
          ? '知识库中已有更新。本地草稿已保留，请比较版本后处理。'
          : null,
    };
  }
  static async open(
    identity: WorkspaceIdentity,
    note: Note,
    storage: DraftStorage,
    save: Save,
  ): Promise<DraftController> {
    const key = draftKey(identity, note.id);
    const stored = await storage.read(key);
    if (stored && (stored.key !== key || draftKey(stored.identity, stored.base.id) !== key))
      throw new Error('草稿身份校验失败。');
    const intent = stored?.historyIntent;
    if (
      intent &&
      (intent.schema !== 1 ||
        !intent.command?.payload?.command_id ||
        !['notes_restore', 'notes_create'].includes(intent.command.kind) ||
        (intent.command.kind === 'notes_restore' &&
          (intent.command.payload.note.note_id !== note.id ||
            intent.command.payload.note.base_revision_id !== stored.base.revision_id)))
    )
      throw new Error('历史操作记录校验失败，草稿已保留。');
    const dirty =
      stored &&
      (stored.historyIntent || stored.pending || stored.generation !== stored.acknowledged);
    const record: DraftRecord = dirty
      ? stored
      : {
          key,
          identity,
          base: note,
          title: note.title,
          document: note.document,
          generation: 0,
          acknowledged: 0,
          pending: null,
          imageImports: {},
        };
    return new DraftController(
      record,
      storage,
      save,
      !!dirty && !stored.pending && stored.base.revision_id !== note.revision_id,
    );
  }
  getSnapshot = (): DraftSnapshot => this.snapshot;
  subscribe = (callback: () => void): (() => void) => {
    this.listeners.add(callback);
    return () => this.listeners.delete(callback);
  };
  private emit(patch: Partial<DraftSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...patch };
    for (const listener of this.listeners) listener();
  }
  /**
   * The first edit of a debounce window writes at once, so a burst is never
   * unsaved for its whole length; later edits merge into the one waiting record
   * that the debounce flush (or an explicit persist) writes a single time.
   */
  private mergeLocalWrite(): Promise<void> {
    const promise = this.enqueue(this.snapshot.record);
    if (!this.merging) {
      this.merging = true;
      // head=true: the first edit of the window writes at once.
      this.drain(true);
    }
    return promise;
  }
  /**
   * Conflict and error drafts never arm the debounce timer, so their edits keep
   * writing through and stay recoverable without an explicit save.
   */
  private writeThrough(): Promise<void> {
    this.merging = false;
    const promise = this.enqueue(this.snapshot.record);
    this.drain();
    return promise;
  }
  /** Queue the newest record: a waiting write is replaced, never duplicated. */
  private enqueue(record: DraftRecord): Promise<void> {
    if (this.queued) {
      this.queued.record = record;
      return this.queued.promise;
    }
    const ticket = localWrite(record);
    this.queued = ticket;
    return ticket.promise;
  }
  /** Start the waiting write unless one is in flight or a window is holding edits. */
  private drain(head = false): void {
    if (this.writing || (this.merging && !head)) return;
    const ticket = this.queued;
    if (!ticket) return;
    this.queued = null;
    const record = ticket.record;
    // One clone per write instead of one clone per keystroke.
    const payload = structuredClone(record);
    this.writing = ticket;
    this.writingRun = Promise.resolve()
      .then(() => this.storage.write(payload))
      .then(
        () => {
          ticket.resolve();
        },
        (error: unknown) => {
          // The draft stays in memory; callers surface the failure and can retry.
          ticket.reject(error);
        },
      )
      .then(() => {
        this.writing = null;
        this.writingRun = null;
        this.drain();
      });
  }
  /**
   * Durable write of the newest record. Every IPC call site awaits this, so the
   * exact payload it is about to send is already on disk.
   */
  private persist(): Promise<void> {
    this.merging = false;
    const record = this.snapshot.record;
    if (this.writing?.record === record) return this.writing.promise;
    const promise = this.enqueue(record);
    this.drain();
    return promise;
  }
  /** Merged edits a draft replacement makes moot; their waiters settle here. */
  private dropQueued(): void {
    const ticket = this.queued;
    if (!ticket) return;
    this.queued = null;
    ticket.resolve();
  }
  /** Resolve once no local write is waiting or running. */
  private async settled(): Promise<void> {
    while (this.writingRun) await this.writingRun;
  }
  update(title: string, document: NoteDocument): void {
    this.assertEditable();
    const record = {
      ...this.snapshot.record,
      title,
      document,
      generation: this.snapshot.record.generation + 1,
    };
    const blocked = this.snapshot.status === 'conflict' || this.snapshot.status === 'error';
    this.emit({
      record,
      status: blocked ? this.snapshot.status : this.running ? 'saving' : 'dirty',
    });
    void (blocked ? this.writeThrough() : this.mergeLocalWrite())
      .then(() => {
        if (!blocked && !this.running && this.snapshot.record.generation === record.generation)
          this.emit({ status: 'local' });
      })
      .catch((error) => this.failed(error));
    if (!blocked) this.schedule();
  }
  async addImageImport(hash: string, commandId: string): Promise<void> {
    this.assertEditable();
    this.emit({
      record: {
        ...this.snapshot.record,
        imageImports: { ...this.snapshot.record.imageImports, [hash]: commandId },
      },
    });
    try {
      await this.persist();
    } catch (error) {
      this.failed(error);
      throw error;
    }
  }
  setComposing(pane: string, value: boolean): void {
    if (value) {
      if (this.adopting) throw new Error('版本操作正在写入，请稍候再编辑。');
      this.composing.add(pane);
      clearTimeout(this.timer);
    } else {
      this.composing.delete(pane);
      this.schedule();
    }
  }
  private schedule(): void {
    clearTimeout(this.timer);
    if (
      !this.stopped &&
      !this.frozen &&
      !this.snapshot.historyBusy &&
      !this.snapshot.record.historyIntent &&
      !this.composing.size &&
      !['error', 'conflict'].includes(this.snapshot.status)
    ) {
      this.timer = setTimeout(() => {
        void this.flush().catch(() => undefined);
      }, 650);
    }
  }
  private failed(error: unknown): void {
    this.emit({
      status: conflict(error) ? 'conflict' : 'error',
      error: error instanceof Error ? error.message : '保存失败，草稿已保留，请重试。',
    });
  }
  async flush(): Promise<void> {
    if (this.adopting) throw new Error('版本操作正在写入，请稍候再保存。');
    if (this.snapshot.record.historyIntent) throw new Error('请先确认待处理的版本操作。');
    clearTimeout(this.timer);
    if (this.composing.size) throw new Error('请先完成当前输入法输入。');
    if (this.snapshot.status === 'conflict')
      throw new Error(this.snapshot.error ?? '笔记存在版本冲突。');
    if (this.running) return this.running;
    // The window ends here; submit() writes the newest record before any IPC.
    this.merging = false;
    this.running = this.submit();
    try {
      await this.running;
    } finally {
      this.running = null;
    }
  }
  private async submit(): Promise<void> {
    try {
      while (!this.stopped && !this.composing.size) {
        let record = this.snapshot.record;
        if (!record.pending && record.generation === record.acknowledged) {
          await this.persist();
          if (this.snapshot.record !== record) continue;
          this.emit({ status: 'saved', error: null });
          return;
        }
        if (!record.pending) {
          record = {
            ...record,
            pending: {
              commandId: crypto.randomUUID(),
              generation: record.generation,
              request: {
                note_id: record.base.id,
                base_revision_id: record.base.revision_id,
                title: record.title.trim() || '未命名笔记',
                document: record.document,
                document_schema: EDITOR_DOCUMENT_SCHEMA,
              },
            },
          };
          this.emit({ record });
        }
        // Persist both the exact retry payload and newer edits before sending.
        await this.persist();
        if (this.composing.size) return;
        this.emit({ status: 'saving', error: null });
        const pending = record.pending!;
        const result = await this.save(pending.commandId, structuredClone(pending.request));
        if (
          result.receipt.command_id !== pending.commandId ||
          result.note.id !== record.base.id ||
          result.receipt.revision_id !== result.note.revision_id
        )
          throw new Error('保存回执校验失败，草稿已保留。');
        record = {
          ...this.snapshot.record,
          base: result.note,
          document: mergeCommittedBindings(
            this.snapshot.record.document,
            pending.request.document,
            result.note.document,
          ),
          acknowledged: pending.generation,
          pending: null,
        };
        this.emit({
          record,
          status: record.generation === record.acknowledged ? 'local' : 'dirty',
        });
        await this.persist();
      }
    } catch (error) {
      // Validation errors are definitive non-commits. A corrected draft needs a
      // new intent; unknown outcomes above still replay the exact old request.
      if (rejectedInput(error) && this.snapshot.record.pending) {
        this.emit({ record: { ...this.snapshot.record, pending: null } });
        try {
          await this.persist();
        } catch (storageError) {
          this.failed(storageError);
          throw storageError;
        }
      }
      this.failed(error);
      throw error;
    }
  }
  async retry(): Promise<void> {
    if (this.adopting) throw new Error('版本操作正在写入，请稍候再保存。');
    if (this.snapshot.status === 'conflict') throw new Error('请先处理版本冲突。');
    this.emit({ status: 'local', error: null });
    await this.flush();
  }
  /** Caller must preserve local text or obtain explicit confirmation before replacing it. */
  async adopt(note: Note, expected: DraftRecord = this.snapshot.record): Promise<void> {
    this.assertEditable();
    if (this.composing.size) throw new Error('请先完成当前输入法输入，再采用版本。');
    if (this.running) throw new Error('请等待当前保存完成。');
    if (this.snapshot.record !== expected)
      throw new Error('操作期间内容又有修改，已保留当前草稿。');
    const old = this.snapshot.record;
    const actions = this.actions;
    if (note.id !== old.base.id) throw new Error('笔记身份不一致。');
    const record = {
      ...old,
      base: note,
      title: note.title,
      document: note.document,
      generation: 0,
      acknowledged: 0,
      pending: null,
    };
    // The adopted revision replaces this draft wholesale, so merged edits that
    // never reached disk are dropped instead of queued behind the adoption.
    this.merging = false;
    this.dropQueued();
    await this.settled();
    this.assertEditable();
    if (this.snapshot.record !== old || this.composing.size)
      throw new Error('内容又有修改，已保留当前草稿，请重新操作。');
    if (this.actions !== actions || this.running)
      throw new Error('图片或保存操作已开始，请完成后重新采用版本。');
    // No edit may be accepted between the final check and durable adoption.
    // Both panes already render historyBusy sessions as read-only.
    this.adopting = true;
    clearTimeout(this.timer);
    this.emit({ historyBusy: true });
    try {
      await this.storage.write(record);
      this.emit({ record, status: 'saved', error: null });
    } finally {
      this.adopting = false;
      this.emit({ historyBusy: false });
    }
  }

  /**
   * Apply a note fetched after a sync pull. A saved editor adopts the new head
   * immediately; any local edit, pending save, composition, image action or
   * history operation keeps the draft untouched and turns it into an explicit
   * conflict so neither side is silently overwritten.
   */
  async applyRemote(note: Note): Promise<'unchanged' | 'adopted' | 'conflict'> {
    const expected = this.snapshot.record;
    if (note.id !== expected.base.id) throw new Error('笔记身份不一致。');
    if (note.revision_id === expected.base.revision_id && note.trashed === expected.base.trashed)
      return 'unchanged';
    const localWork =
      this.snapshot.status !== 'saved' ||
      expected.pending !== null ||
      expected.generation !== expected.acknowledged ||
      expected.historyIntent !== undefined ||
      this.running !== null ||
      this.adopting ||
      this.composing.size > 0 ||
      this.actions > 0 ||
      this.queued !== null ||
      this.writing !== null;
    if (localWork || note.trashed) {
      this.markRemoteConflict(note);
      return 'conflict';
    }
    try {
      await this.adopt(note, expected);
      return this.snapshot.record.base.revision_id === note.revision_id ? 'adopted' : 'conflict';
    } catch {
      this.markRemoteConflict(note);
      return 'conflict';
    }
  }

  private markRemoteConflict(note: Note): void {
    this.emit({
      status: 'conflict',
      error: note.trashed
        ? '这篇笔记已在其他设备移入回收站。本地草稿已保留，未自动删除。'
        : '知识库中已有来自其他设备的更新。本地草稿已保留，请比较版本后处理。',
    });
  }
  /** Discard only the compared conflict draft; adopting never writes a core revision. */
  async discardConflict(
    expected: DraftRecord,
    comparedRevision: string,
    loadCurrent: () => Promise<Note>,
  ): Promise<void> {
    this.assertEditable();
    if (this.snapshot.status !== 'conflict' || this.snapshot.record !== expected)
      throw new Error('本地草稿已变化，请重新比较后确认。');
    if (this.composing.size) throw new Error('请先完成当前输入法输入。');
    if (this.actions || this.running) throw new Error('请等待图片或保存操作完成。');
    const finish = this.beginAction();
    try {
      const current = await loadCurrent();
      if (current.id !== expected.base.id || current.revision_id !== comparedRevision)
        throw new Error('知识库版本已变化，请重新比较后确认。');
      if (current.trashed) throw new Error('笔记已在回收站，草稿已保留。');
      if (this.actions !== 1) throw new Error('图片操作已开始，请完成后重新采用版本。');
      await this.adopt(current, expected);
    } finally {
      finish();
    }
  }
  /** Durable history commands share the same record and lock in both panes. */
  async runHistory(
    send: (command: HistoryCommand) => Promise<NoteCommit>,
    loadCurrent: () => Promise<Note>,
    operation?: { restoreRevision: string } | { conflictSuffix: string },
  ): Promise<void> {
    if (this.snapshot.historyBusy) throw new Error('版本操作正在进行，请稍候。');
    if (this.frozen || this.stopped) throw new Error('编辑会话正在关闭。');
    if (this.composing.size) throw new Error('请先完成当前输入法输入。');
    if (this.actions || this.running) throw new Error('请等待图片或保存操作完成。');
    if (operation && this.snapshot.record.historyIntent)
      throw new Error('请先确认待处理的版本操作。');
    if (!operation && !this.snapshot.record.historyIntent) return;
    clearTimeout(this.timer);
    this.emit({ historyBusy: true, error: null });
    let sent = false;
    let committed = false;
    try {
      if (operation) {
        if ('restoreRevision' in operation) await this.flush();
        else if (this.snapshot.status !== 'conflict')
          throw new Error('当前草稿没有已确认的版本冲突，请先重试保存。');
        const record = this.snapshot.record;
        let command: HistoryCommand;
        if ('restoreRevision' in operation) {
          command = {
            kind: 'notes_restore',
            payload: {
              command_id: crypto.randomUUID(),
              note: {
                note_id: record.base.id,
                base_revision_id: record.base.revision_id,
                target_revision_id: operation.restoreRevision,
              },
            },
          };
        } else {
          const document = structuredClone(record.document);
          const remap = (nodes: typeof document.content) => {
            for (const node of nodes) {
              if (node.attrs?.id) node.attrs.id = crypto.randomUUID();
              if (node.attrs?.occurrence_id) node.attrs.occurrence_id = crypto.randomUUID();
              if (node.content) remap(node.content);
            }
          };
          remap(document.content);
          command = {
            kind: 'notes_create',
            payload: {
              command_id: crypto.randomUUID(),
              note: {
                // The core caps titles at 300 characters, so a long title plus the
                // suffix must be truncated here instead of failing the whole save.
                title: duplicateTitle(record.title, operation.conflictSuffix),
                document_schema: EDITOR_DOCUMENT_SCHEMA,
                document,
              },
            },
          };
        }
        this.emit({ record: { ...record, historyIntent: { schema: 1, command } } });
      }
      const record = this.snapshot.record;
      const command = record.historyIntent!.command;
      // No IPC until the exact intent and preserved draft have reached disk.
      await this.persist();
      sent = true;
      const result = await send(structuredClone(command));
      if (
        result.receipt.command_id !== command.payload.command_id ||
        result.receipt.object_id !== result.note.id ||
        result.receipt.revision_id !== result.note.revision_id ||
        (command.kind === 'notes_restore' && result.note.id !== record.base.id) ||
        (command.kind === 'notes_create' && result.note.id === record.base.id)
      )
        throw new Error('版本操作回执校验失败，草稿与原请求已保留。');
      committed = true;
      // A replay may return an older receipt after another client has edited.
      const current = await loadCurrent();
      if (current.id !== record.base.id) throw new Error('笔记身份不一致，草稿已保留。');
      const adopted: DraftRecord = {
        ...record,
        base: current,
        title: current.title,
        document: current.document,
        generation: 0,
        acknowledged: 0,
        pending: null,
        historyIntent: null,
      };
      // Clearing the intent and replacing the draft must be one durable write.
      await this.settled();
      this.merging = false;
      await this.storage.write(adopted);
      this.emit({ record: adopted, status: 'saved', error: null });
    } catch (error) {
      const code =
        error && typeof error === 'object' && 'details' in error
          ? (error.details as { code?: string } | null)?.code
          : undefined;
      if (
        sent &&
        !committed &&
        code &&
        [
          'revision_conflict',
          'invalid_input',
          'invalid_document',
          'object_trashed',
          'not_found',
        ].includes(code)
      ) {
        const previous = this.snapshot.record;
        const cleared = { ...previous, historyIntent: null };
        try {
          await this.settled();
          await this.storage.write(cleared);
          this.emit({ record: cleared });
        } catch (storageError) {
          this.failed(storageError);
          throw storageError;
        }
      }
      this.failed(error);
      throw error;
    } finally {
      this.emit({ historyBusy: false });
    }
  }

  async dispose(): Promise<void> {
    clearTimeout(this.timer);
    await this.flush();
    this.stopped = true;
    this.listeners.clear();
  }

  private assertEditable(): void {
    if (this.snapshot.historyBusy || this.snapshot.record.historyIntent)
      throw new Error('请先确认待处理的版本操作。');
    if (this.stopped) throw new Error('编辑会话已关闭。');
    if (this.frozen) throw new Error('正在保存并切换工作区，请稍候。');
  }

  /** Image/history operations must finish before a workspace can be closed. */
  beginAction(): () => void {
    this.assertEditable();
    this.actions += 1;
    let finished = false;
    return () => {
      if (!finished) this.actions -= 1;
      finished = true;
    };
  }

  freeze(): () => void {
    if (this.snapshot.historyBusy || this.snapshot.record.historyIntent)
      throw new Error('请先确认待处理的版本操作。');
    if (this.composing.size) throw new Error('请先完成当前输入法输入。');
    if (this.actions) throw new Error('请等待图片或版本操作完成后再切换工作区。');
    this.frozen = true;
    return () => {
      this.frozen = false;
    };
  }

  /** Release only after a frozen flush; never sends commands to a closed workspace. */
  release(): void {
    if (!this.frozen || this.running || this.snapshot.status !== 'saved')
      throw new Error('尚有未保存的编辑，不能关闭会话。');
    clearTimeout(this.timer);
    this.stopped = true;
    this.listeners.clear();
  }
}

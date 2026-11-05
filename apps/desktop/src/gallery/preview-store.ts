import type { Note, NoteSummary, WorkspaceIdentity } from '@pkms/contracts';
import { projectPreview, type PreviewDocument } from './preview-document.ts';
import { SUPPORTED_DOCUMENT_SCHEMAS } from '../editor/document-schema.ts';

export type PreviewRequest = {
  identity: WorkspaceIdentity;
  noteId: string;
  revisionId: string;
  schemaVersion: number;
};
export type PreviewResult =
  | { status: 'loading' }
  | { status: 'ready'; document: PreviewDocument }
  | { status: 'error'; error: string };
type Finished = Exclude<PreviewResult, { status: 'loading' }>;
type Listener = (result: PreviewResult) => void;
type Pending = { key: string; request: PreviewRequest; listeners: Set<Listener>; running: boolean };
type Cached = { value: Finished; bytes: number };

export function previewRequest(identity: WorkspaceIdentity, note: NoteSummary): PreviewRequest {
  return {
    identity: { ...identity },
    noteId: note.id,
    revisionId: note.revision_id,
    schemaVersion: note.document_schema,
  };
}
export function previewKey(request: PreviewRequest): string {
  return JSON.stringify([
    request.identity.local_instance_id,
    request.identity.workspace_id,
    request.noteId,
    request.revisionId,
    request.schemaVersion,
  ]);
}

/** Bounded projected data only. In-flight IPC cannot be aborted; obsolete results are discarded. */
export class PreviewStore {
  private readonly read: (request: PreviewRequest) => Promise<Note>;
  private readonly maxEntries: number;
  private readonly maxBytes: number;
  private readonly concurrent: number;
  private readonly timeoutMs: number;
  private cache = new Map<string, Cached>();
  private pending = new Map<string, Pending>();
  private queue: Pending[] = [];
  private running = 0;
  private bytes = 0;

  constructor(
    read: (request: PreviewRequest) => Promise<Note>,
    limits: { entries?: number; bytes?: number; concurrent?: number; timeoutMs?: number } = {},
  ) {
    this.read = read;
    this.maxEntries = limits.entries ?? 64;
    this.maxBytes = limits.bytes ?? 512 * 1024;
    this.concurrent = limits.concurrent ?? 2;
    this.timeoutMs = limits.timeoutMs ?? 8_000;
  }

  subscribe(request: PreviewRequest, listener: Listener): () => void {
    const key = previewKey(request),
      cached = this.cache.get(key);
    if (cached) {
      this.cache.delete(key);
      this.cache.set(key, cached);
      listener(cached.value);
      return () => undefined;
    }
    let entry = this.pending.get(key);
    if (!entry) {
      entry = {
        key,
        request: { ...request, identity: { ...request.identity } },
        listeners: new Set(),
        running: false,
      };
      this.pending.set(key, entry);
      this.queue.push(entry);
    }
    entry.listeners.add(listener);
    listener({ status: 'loading' });
    this.pump();
    return () => {
      entry.listeners.delete(listener);
      if (entry.listeners.size || this.pending.get(key) !== entry) return;
      this.pending.delete(key);
      if (!entry.running) this.queue = this.queue.filter((queued) => queued !== entry);
    };
  }

  retry(request: PreviewRequest): void {
    const key = previewKey(request),
      cached = this.cache.get(key);
    if (cached?.value.status === 'error') {
      this.cache.delete(key);
      this.bytes -= cached.bytes;
    }
  }

  stats() {
    return {
      running: this.running,
      queued: this.queue.length,
      entries: this.cache.size,
      bytes: this.bytes,
    };
  }

  private remember(key: string, value: Finished) {
    const bytes = new TextEncoder().encode(key + JSON.stringify(value)).byteLength;
    if (bytes > this.maxBytes) return;
    const previous = this.cache.get(key);
    if (previous) {
      this.bytes -= previous.bytes;
      this.cache.delete(key);
    }
    while (this.cache.size >= this.maxEntries || this.bytes + bytes > this.maxBytes) {
      const oldest = this.cache.entries().next().value;
      if (!oldest) break;
      this.cache.delete(oldest[0]);
      this.bytes -= oldest[1].bytes;
    }
    this.cache.set(key, { value, bytes });
    this.bytes += bytes;
  }

  private readWithTimeout(request: PreviewRequest): Promise<Note> {
    if (this.timeoutMs <= 0) return this.read(request);
    return new Promise<Note>((resolve, reject) => {
      const timer = globalThis.setTimeout(
        () => reject(new Error('预览读取超时，请重试。 / Preview timed out. Retry to load it.')),
        this.timeoutMs,
      );
      this.read(request).then(
        (note) => {
          globalThis.clearTimeout(timer);
          resolve(note);
        },
        (error: unknown) => {
          globalThis.clearTimeout(timer);
          reject(error);
        },
      );
    });
  }

  private pump() {
    while (this.running < this.concurrent && this.queue.length) {
      const entry = this.queue.shift()!;
      if (this.pending.get(entry.key) !== entry || !entry.listeners.size) continue;
      ++this.running;
      entry.running = true;
      const current = () => this.pending.get(entry.key) === entry && entry.listeners.size > 0;
      const publish = (result: Finished) => {
        if (!current()) return;
        this.remember(entry.key, result);
        for (const listener of entry.listeners) listener(result);
      };
      void Promise.resolve()
        .then(() => (current() ? this.readWithTimeout(entry.request) : null))
        .then((note) => {
          if (!note || !current()) return;
          if (
            note.id !== entry.request.noteId ||
            note.revision_id !== entry.request.revisionId ||
            note.schema_version !== entry.request.schemaVersion
          )
            throw new Error('预览版本不匹配，已停止显示。 / Preview revision does not match.');
          if (!SUPPORTED_DOCUMENT_SCHEMAS.includes(note.schema_version))
            throw new Error(
              '此正文版本暂不支持预览。 / This document version cannot be previewed.',
            );
          publish({ status: 'ready', document: projectPreview(note.document) });
        })
        .catch((error: unknown) => {
          publish({
            status: 'error',
            error:
              error instanceof Error
                ? error.message.slice(0, 512)
                : '预览读取失败。 / Preview could not be read.',
          });
        })
        .finally(() => {
          if (this.pending.get(entry.key) === entry) this.pending.delete(entry.key);
          --this.running;
          this.pump();
        });
    }
  }
}

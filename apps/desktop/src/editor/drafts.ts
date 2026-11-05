import type {
  DesktopCommand,
  Note,
  NoteDocument,
  SaveNote,
  WorkspaceIdentity,
} from '@pkms/contracts';

export interface SaveAttempt {
  commandId: string;
  generation: number;
  request: SaveNote;
}
export type HistoryCommand = Extract<DesktopCommand, { kind: 'notes_restore' | 'notes_create' }>;
export interface HistoryIntent {
  schema: 1;
  command: HistoryCommand;
}
export interface DraftRecord {
  key: string;
  identity: WorkspaceIdentity;
  base: Note;
  title: string;
  document: NoteDocument;
  generation: number;
  acknowledged: number;
  pending: SaveAttempt | null;
  imageImports: Record<string, string>;
  historyIntent?: HistoryIntent | null;
}
export interface DraftStorage {
  read(key: string): Promise<DraftRecord | undefined>;
  write(record: DraftRecord): Promise<void>;
}
export function draftKey(identity: WorkspaceIdentity, noteId: string): string {
  return `${identity.local_instance_id}/${identity.workspace_id}/${noteId}`;
}
let database: Promise<IDBDatabase> | undefined;
function open(): Promise<IDBDatabase> {
  return (database ??= new Promise((resolve, reject) => {
    const request = indexedDB.open('pkms-local-drafts', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('drafts', { keyPath: 'key' });
    request.onerror = () => {
      database = undefined;
      reject(new Error('无法打开本地草稿存储，请检查磁盘空间。'));
    };
    request.onblocked = () => {
      database = undefined;
      reject(new Error('草稿存储被旧窗口占用，请关闭其他 spanseek 窗口。'));
    };
    request.onsuccess = () => {
      request.result.onversionchange = () => {
        request.result.close();
        database = undefined;
      };
      resolve(request.result);
    };
  }));
}
export const indexedDrafts: DraftStorage = {
  async read(key) {
    const db = await open();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction('drafts', 'readonly');
      const request = transaction.objectStore('drafts').get(key);
      transaction.oncomplete = () => resolve(request.result);
      transaction.onabort = transaction.onerror = () =>
        reject(new Error('读取本地草稿失败。为保护编辑内容，已停止打开笔记。'));
    });
  },
  async write(record) {
    const db = await open();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction('drafts', 'readwrite', { durability: 'strict' });
      transaction.objectStore('drafts').put(record);
      transaction.oncomplete = () => resolve();
      transaction.onabort = transaction.onerror = () =>
        reject(new Error('本地草稿未能落盘，请保留窗口并检查磁盘空间。'));
    });
  },
};

/** Called only after an acknowledged permanent-delete commit, never for soft Trash. */
export async function deleteDraftRecords(
  identity: WorkspaceIdentity,
  noteIds: string[],
): Promise<void> {
  if (noteIds.length === 0) return;
  const db = await open();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction('drafts', 'readwrite', { durability: 'strict' });
    const store = transaction.objectStore('drafts');
    for (const noteId of new Set(noteIds)) store.delete(draftKey(identity, noteId));
    transaction.oncomplete = () => resolve();
    transaction.onabort = transaction.onerror = () =>
      reject(new Error('内容已彻底删除，本地草稿清理尚未完成，请保留窗口并重试确认。'));
  });
}

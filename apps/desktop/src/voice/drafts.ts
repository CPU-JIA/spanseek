// Crash-safe recording drafts live in IndexedDB, outside SQLite, until the
// capture is committed. A crash leaves the chunks recoverable on the next open.
const DATABASE = 'spanseek-voice-drafts';
const VERSION = 1;
const DRAFTS = 'drafts';
const CHUNKS = 'chunks';

export type VoiceDraft = {
  id: string;
  startedAt: number;
  updatedAt: number;
  mimeType: string;
  bytes: number;
};

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, VERSION);
    request.onupgradeneeded = () => {
      const database = request.result;
      if (!database.objectStoreNames.contains(DRAFTS)) {
        database.createObjectStore(DRAFTS, { keyPath: 'id' });
      }
      if (!database.objectStoreNames.contains(CHUNKS)) {
        const store = database.createObjectStore(CHUNKS, { autoIncrement: true });
        store.createIndex('draftId', 'draftId', { unique: false });
      }
    };
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
  });
}

function run<T>(
  mode: IDBTransactionMode,
  action: (tx: IDBTransaction) => Promise<T> | T,
): Promise<T> {
  return open().then(
    (database) =>
      new Promise<T>((resolve, reject) => {
        const tx = database.transaction([DRAFTS, CHUNKS], mode);
        let result: T;
        Promise.resolve(action(tx))
          .then((value) => {
            result = value;
            tx.oncomplete = () => {
              database.close();
              resolve(result);
            };
            tx.onerror = () => {
              database.close();
              reject(tx.error);
            };
            tx.onabort = () => {
              database.close();
              reject(tx.error ?? new Error('voice draft transaction aborted'));
            };
          })
          .catch((error) => {
            database.close();
            reject(error);
          });
      }),
  );
}

export async function startDraft(id: string, mimeType: string): Promise<void> {
  const now = Date.now();
  await run('readwrite', (tx) => {
    tx.objectStore(DRAFTS).put({
      id,
      startedAt: now,
      updatedAt: now,
      mimeType,
      bytes: 0,
    } satisfies VoiceDraft);
  });
}

export async function appendDraftChunk(id: string, chunk: Blob): Promise<void> {
  if (chunk.size === 0) return;
  const bytes = await chunk.arrayBuffer();
  await run('readwrite', (tx) => {
    const drafts = tx.objectStore(DRAFTS);
    tx.objectStore(CHUNKS).add({ draftId: id, bytes });
    const read = drafts.get(id);
    read.onsuccess = () => {
      const draft = read.result as VoiceDraft | undefined;
      if (!draft) return;
      draft.updatedAt = Date.now();
      draft.bytes += bytes.byteLength;
      drafts.put(draft);
    };
  });
}

export async function listDrafts(): Promise<VoiceDraft[]> {
  return run('readonly', (tx) => {
    return new Promise<VoiceDraft[]>((resolve, reject) => {
      const request = tx.objectStore(DRAFTS).getAll();
      request.onsuccess = () =>
        resolve((request.result as VoiceDraft[]).sort((a, b) => b.startedAt - a.startedAt));
      request.onerror = () => reject(request.error);
    });
  });
}

export async function loadDraft(id: string): Promise<Blob | null> {
  return run('readonly', (tx) => {
    return new Promise<Blob | null>((resolve, reject) => {
      const read = tx.objectStore(DRAFTS).get(id);
      read.onsuccess = () => {
        const draft = read.result as VoiceDraft | undefined;
        if (!draft) {
          resolve(null);
          return;
        }
        const chunks = tx.objectStore(CHUNKS).index('draftId').getAll(IDBKeyRange.only(id));
        chunks.onsuccess = () => {
          const buffers = (chunks.result as { bytes: ArrayBuffer }[]).map((item) => item.bytes);
          resolve(new Blob(buffers, { type: draft.mimeType }));
        };
        chunks.onerror = () => reject(chunks.error);
      };
      read.onerror = () => reject(read.error);
    });
  });
}

export async function deleteDraft(id: string): Promise<void> {
  await run('readwrite', (tx) => {
    tx.objectStore(DRAFTS).delete(id);
    const index = tx.objectStore(CHUNKS).index('draftId');
    const request = index.openCursor(IDBKeyRange.only(id));
    request.onsuccess = () => {
      const cursor = request.result;
      if (!cursor) return;
      cursor.delete();
      cursor.continue();
    };
  });
}

export function draftDurationMs(draft: VoiceDraft): number {
  return Math.max(0, draft.updatedAt - draft.startedAt);
}

import type { WorkspaceIdentity } from '@pkms/contracts';
import {
  annotationNoteItems,
  annotationNoteKey,
  pendingAnnotationNoteAction,
  validateAnnotationNoteAction,
  type AnnotationNoteAction,
} from './annotation-to-note';
let connection: Promise<IDBDatabase> | undefined;
function database(): Promise<IDBDatabase> {
  return (connection ??= new Promise((resolve, reject) => {
    const request = indexedDB.open('pkms-annotation-note-intents', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('intents', { keyPath: 'key' });
    request.onerror = request.onblocked = () => {
      connection = undefined;
      reject(new Error('无法打开摘录操作记录。 / Cannot open excerpt actions.'));
    };
    request.onsuccess = () => {
      const db = request.result;
      db.onversionchange = () => {
        db.close();
        connection = undefined;
      };
      resolve(db);
    };
  }));
}
export const annotationNoteStorage = {
  async read(key: string): Promise<AnnotationNoteAction | undefined> {
    const db = await database();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('intents', 'readonly');
      const request = tx.objectStore('intents').get(key);
      tx.oncomplete = () => resolve(request.result);
      tx.onerror = tx.onabort = () =>
        reject(new Error('无法读取摘录操作。 / Cannot read excerpt action.'));
    });
  },
  async readSelection(
    identity: WorkspaceIdentity,
    annotationIds: string[],
  ): Promise<AnnotationNoteAction | undefined> {
    const db = await database();
    const actions = await new Promise<AnnotationNoteAction[]>((resolve, reject) => {
      const tx = db.transaction('intents', 'readonly');
      const prefix = annotationNoteKey(identity, '');
      const request = tx
        .objectStore('intents')
        .getAll(IDBKeyRange.bound(prefix, `${prefix}\uffff`));
      tx.oncomplete = () => resolve(request.result);
      tx.onerror = tx.onabort = () =>
        reject(new Error('无法读取摘录操作。 / Cannot read excerpt actions.'));
    });
    return pendingAnnotationNoteAction(actions, identity, annotationIds);
  },
  async begin(intent: AnnotationNoteAction): Promise<AnnotationNoteAction> {
    validateAnnotationNoteAction(intent, intent.identity);
    const db = await database();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      const store = tx.objectStore('intents');
      const prefix = annotationNoteKey(intent.identity, '');
      const request = store.getAll(IDBKeyRange.bound(prefix, `${prefix}\uffff`));
      let chosen = intent;
      let failure: unknown;
      request.onsuccess = () => {
        try {
          const existing = pendingAnnotationNoteAction(
            request.result,
            intent.identity,
            annotationNoteItems(intent).map((item) => item.annotationId),
          );
          if (existing) chosen = existing;
          else store.put(intent);
        } catch (error) {
          failure = error;
          tx.abort();
        }
      };
      tx.oncomplete = () => resolve(chosen);
      tx.onerror = tx.onabort = () =>
        reject(
          failure ??
            new Error(
              '无法暂存摘录操作，尚未修改笔记。 / Cannot persist excerpt action; the note was not changed.',
            ),
        );
    });
  },
  async remove(intent: AnnotationNoteAction): Promise<void> {
    const db = await database();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      const store = tx.objectStore('intents');
      const request = store.get(intent.key);
      request.onsuccess = () => {
        if (request.result?.occurrenceId === intent.occurrenceId) store.delete(intent.key);
      };
      tx.oncomplete = () => resolve();
      tx.onerror = tx.onabort = () =>
        reject(
          new Error(
            '保存回执尚未完成记录，请重试。 / Save confirmation could not be recorded. Retry this action.',
          ),
        );
    });
  },
};

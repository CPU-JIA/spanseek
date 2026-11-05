import type { TrashIntent, TrashIntentStorage } from './trash-actions';

let connection: Promise<IDBDatabase> | undefined;
function database() {
  return (connection ??= new Promise<IDBDatabase>((resolve, reject) => {
    // Keep the established technical prefix; branding never changes storage identity.
    const request = indexedDB.open('pkms-trash-lifecycle', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('intents', { keyPath: 'key' });
    request.onerror = request.onblocked = () => {
      connection = undefined;
      reject(new Error('Trash operation storage unavailable / 回收站操作记录不可用'));
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
export const trashIntentStorage: TrashIntentStorage = {
  async read(key) {
    const db = await database();
    return new Promise<TrashIntent | undefined>((resolve, reject) => {
      const tx = db.transaction('intents', 'readonly');
      const request = tx.objectStore('intents').get(key);
      tx.oncomplete = () => resolve(request.result);
      tx.onabort = tx.onerror = () =>
        reject(new Error('Cannot read Trash operation / 无法读取回收站待确认操作'));
    });
  },
  async write(intent) {
    const db = await database();
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      tx.objectStore('intents').put(intent);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(new Error('Cannot save Trash operation; retry / 无法暂存回收站操作，请重试原操作'));
    });
  },
  async remove(key) {
    const db = await database();
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      tx.objectStore('intents').delete(key);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(new Error('Cannot record confirmation; retry / 无法记录确认结果，请重试原操作'));
    });
  },
};

import type { CreateIntent, CreateStorage } from './create-actions';
let connection: Promise<IDBDatabase> | undefined;
function database() {
  return (connection ??= new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('pkms-note-create-intents', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('intents', { keyPath: 'key' });
    request.onerror = request.onblocked = () => {
      connection = undefined;
      reject(new Error('Cannot open note creation records / 无法打开新建笔记记录'));
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
export const createStorage: CreateStorage = {
  async read(key) {
    const db = await database();
    return new Promise<CreateIntent | undefined>((resolve, reject) => {
      const tx = db.transaction('intents', 'readonly');
      const request = tx.objectStore('intents').get(key);
      tx.oncomplete = () => resolve(request.result);
      tx.onabort = tx.onerror = () =>
        reject(new Error('Cannot read note creation records / 无法读取新建笔记记录'));
    });
  },
  async write(intent) {
    const db = await database();
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      tx.objectStore('intents').put(intent);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(new Error('Cannot persist note creation / 无法暂存新建笔记操作'));
    });
  },
  async remove(key) {
    const db = await database();
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      tx.objectStore('intents').delete(key);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(new Error('Cannot confirm the opened note / 无法记录笔记已打开，请继续原操作'));
    });
  },
};

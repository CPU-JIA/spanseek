import type { PageIntentStorage } from './page-actions.ts';

let connection: Promise<IDBDatabase> | undefined;
function database() {
  return (connection ??= new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('pkms-organization-page-intents', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('intents', { keyPath: 'key' });
    request.onerror = request.onblocked = () => {
      connection = undefined;
      reject(new Error('Local operation storage unavailable / 本机操作记录不可用'));
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
export const pageIntentStorage: PageIntentStorage = {
  async read(key) {
    const db = await database();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('intents', 'readonly');
      const request = tx.objectStore('intents').get(key);
      tx.oncomplete = () => resolve(request.result);
      tx.onabort = tx.onerror = () =>
        reject(new Error('Cannot read pending operation / 无法读取待确认操作'));
    });
  },
  async write(intent) {
    const db = await database();
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      tx.objectStore('intents').put(intent);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(new Error('Operation not sent: local storage failed / 操作未发送：本机暂存失败'));
    });
  },
  async remove(key) {
    const db = await database();
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      tx.objectStore('intents').delete(key);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(
          new Error(
            'Cannot record confirmation; retry the same operation / 无法记录确认结果，请重试原操作',
          ),
        );
    });
  },
};

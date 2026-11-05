import type { SourceBulkRecord, SourceBulkStorage } from './source-bulk-actions';

let connection: Promise<IDBDatabase> | undefined;
function database() {
  return (connection ??= new Promise((resolve, reject) => {
    const request = indexedDB.open('pkms-source-batches', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('batches', { keyPath: 'key' });
    request.onerror = request.onblocked = () => {
      connection = undefined;
      reject(new Error('无法打开资料整理记录 / Cannot open document batches'));
    };
    request.onsuccess = () => {
      request.result.onversionchange = () => {
        request.result.close();
        connection = undefined;
      };
      resolve(request.result);
    };
  }));
}
export const sourceBulkStorage: SourceBulkStorage = {
  async read(key) {
    const db = await database();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('batches', 'readonly');
      const request = tx.objectStore('batches').get(key);
      tx.oncomplete = () => resolve(request.result);
      tx.onerror = tx.onabort = () =>
        reject(new Error('无法读取资料整理记录 / Cannot read document batches'));
    });
  },
  write: (record) => change(record, false),
  remove: (record) => change(record, true),
};
async function change(record: SourceBulkRecord, remove: boolean) {
  const db = await database();
  return new Promise<void>((resolve, reject) => {
    const tx = db.transaction('batches', 'readwrite', { durability: 'strict' });
    const store = tx.objectStore('batches');
    const request = store.get(record.key);
    request.onsuccess = () => {
      if (request.result && request.result.id !== record.id) {
        tx.abort();
        return;
      }
      if (remove) store.delete(record.key);
      else store.put(record);
    };
    tx.oncomplete = () => resolve();
    tx.onerror = tx.onabort = () =>
      reject(
        new Error(
          '资料整理记录未能保存，请重试原批次 / Batch storage failed; retry the original batch',
        ),
      );
  });
}

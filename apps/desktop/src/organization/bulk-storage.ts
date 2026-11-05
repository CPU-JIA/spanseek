import type { BulkRecord, BulkStorage } from './bulk-actions';

let connection: Promise<IDBDatabase> | undefined;
function database() {
  return (connection ??= new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('pkms-bulk-organization', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('batches', { keyPath: 'key' });
    request.onerror = request.onblocked = () => {
      connection = undefined;
      reject(new Error('Cannot open organization records / 无法打开整理记录'));
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
export const bulkStorage: BulkStorage = {
  async read(key) {
    const db = await database();
    return new Promise<BulkRecord | undefined>((resolve, reject) => {
      const tx = db.transaction('batches', 'readonly');
      const request = tx.objectStore('batches').get(key);
      tx.oncomplete = () => resolve(request.result);
      tx.onabort = tx.onerror = () =>
        reject(new Error('Cannot read organization records / 无法读取整理记录'));
    });
  },
  async write(record) {
    const db = await database();
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction('batches', 'readwrite', { durability: 'strict' });
      tx.objectStore('batches').put(record);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(new Error('Organization not saved locally / 整理操作未能暂存到本机'));
    });
  },
  async remove(key) {
    const db = await database();
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction('batches', 'readwrite', { durability: 'strict' });
      tx.objectStore('batches').delete(key);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(new Error('Cannot update organization records / 无法更新整理记录'));
    });
  },
};

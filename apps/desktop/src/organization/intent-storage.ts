import type { WorkspaceIdentity } from '@pkms/contracts';
import type { MetadataMutation } from './metadata-controller.ts';

export interface MetadataIntent {
  key: string;
  identity: WorkspaceIdentity;
  objectId: string;
  command: MetadataMutation;
}
export interface IntentStorage {
  read(key: string): Promise<MetadataIntent | undefined>;
  write(intent: MetadataIntent): Promise<void>;
  remove(key: string): Promise<void>;
}
let database: Promise<IDBDatabase> | undefined;
function open(): Promise<IDBDatabase> {
  return (database ??= new Promise((resolve, reject) => {
    const request = indexedDB.open('pkms-metadata-intents', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('intents', { keyPath: 'key' });
    request.onerror = request.onblocked = () => {
      database = undefined;
      reject(new Error('无法打开整理操作记录，请检查本地存储并关闭其他 spanseek 窗口。'));
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
export const indexedIntents: IntentStorage = {
  async read(key) {
    const db = await open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('intents', 'readonly');
      const request = tx.objectStore('intents').get(key);
      tx.oncomplete = () => resolve(request.result);
      tx.onabort = tx.onerror = () => reject(new Error('无法读取上次整理操作，已停止新操作。'));
    });
  },
  async write(intent) {
    const db = await open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      tx.objectStore('intents').put(intent);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(new Error('整理操作未能暂存到本机，尚未发送，请检查磁盘空间。'));
    });
  },
  async remove(key) {
    const db = await open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
      tx.objectStore('intents').delete(key);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(new Error('无法更新整理操作记录，请重新确认上次结果。'));
    });
  },
};

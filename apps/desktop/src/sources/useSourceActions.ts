import { useEffect, useMemo, useSyncExternalStore } from 'react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { SourceActions, intentId, type SourceIntentStorage } from './source-actions';
let database: Promise<IDBDatabase> | undefined;
function db() {
  return (database ??= new Promise((resolve, reject) => {
    const request = indexedDB.open('pkms-source-intents', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('intents', { keyPath: 'key' });
    request.onerror = request.onblocked = () => {
      database = undefined;
      reject(new Error('本机待确认操作记录不可用 / Pending operation storage unavailable'));
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
const storage: SourceIntentStorage = {
  async read(key) {
    const database = await db();
    return new Promise((resolve, reject) => {
      const tx = database.transaction('intents', 'readonly');
      const request = tx.objectStore('intents').get(key);
      tx.oncomplete = () => resolve(request.result);
      tx.onerror = tx.onabort = () => reject(new Error('Cannot read pending source request'));
    });
  },
  async put(intent) {
    const database = await db();
    return new Promise((resolve, reject) => {
      const tx = database.transaction('intents', 'readwrite', { durability: 'strict' });
      const store = tx.objectStore('intents');
      const request = store.get(intent.key);
      request.onsuccess = () => {
        if (request.result) tx.abort();
        else store.put(intent);
      };
      tx.oncomplete = () => resolve();
      tx.onerror = tx.onabort = () =>
        reject(
          new Error('Pending operation exists or storage failed / 待确认操作已存在或本机记录失败'),
        );
    });
  },
  async remove(intent) {
    const database = await db();
    return new Promise((resolve, reject) => {
      const tx = database.transaction('intents', 'readwrite', { durability: 'strict' });
      const store = tx.objectStore('intents');
      const request = store.get(intent.key);
      request.onsuccess = () => {
        if (!request.result) return;
        if (intentId(request.result) !== intentId(intent)) {
          tx.abort();
          return;
        }
        store.delete(intent.key);
      };
      tx.oncomplete = () => resolve();
      tx.onerror = tx.onabort = () =>
        reject(
          new Error(
            'Confirmation cleanup failed; retry original request / 确认记录失败，请重试原请求',
          ),
        );
    });
  },
};
const controllers = new Map<string, SourceActions>();
export function getSourceActions(identity: WorkspaceIdentity, scope: string) {
  const key = JSON.stringify([identity.local_instance_id, identity.workspace_id, scope]);
  let value = controllers.get(key);
  if (!value) {
    value = new SourceActions(key, identity, storage, (intent) =>
      call(intent.command, intent.expected, intent.identity),
    );
    controllers.set(key, value);
  }
  return value;
}
export function useSourceActions(identity: WorkspaceIdentity, scope: string) {
  const key = JSON.stringify([identity.local_instance_id, identity.workspace_id, scope]);
  const controller = useMemo(() => getSourceActions(identity, scope), [key]);
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot,
  );
  useEffect(() => {
    void controller.load();
  }, [controller]);
  return { ...state, execute: controller.execute, retry: controller.retry };
}

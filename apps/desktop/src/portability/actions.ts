import { useEffect, useSyncExternalStore } from 'react';
import { call } from '../api';
import { PendingActions, type PendingAction, type PendingStorage } from './pending-actions';

let database: Promise<IDBDatabase> | undefined;
function open() {
  return (database ??= new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('pkms-portable-requests', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('requests', { keyPath: 'id' });
    request.onerror = request.onblocked = () => {
      database = undefined;
      reject(new Error('无法打开任务请求记录 / Cannot open local task requests.'));
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
async function transaction<T>(
  mode: IDBTransactionMode,
  perform: (store: IDBObjectStore) => IDBRequest<T>,
) {
  const db = await open();
  return new Promise<T>((resolve, reject) => {
    const tx = db.transaction(
      'requests',
      mode,
      mode === 'readwrite' ? { durability: 'strict' } : undefined,
    );
    const request = perform(tx.objectStore('requests'));
    tx.oncomplete = () => resolve(request.result);
    tx.onerror = tx.onabort = () =>
      reject(new Error('无法更新本机任务请求记录 / Local task request storage failed.'));
  });
}
const storage: PendingStorage = {
  read: () => transaction('readonly', (store) => store.getAll()) as Promise<PendingAction[]>,
  put: async (action) => {
    await transaction('readwrite', (store) => store.put(action));
  },
  remove: async (id) => {
    await transaction('readwrite', (store) => store.delete(id));
  },
};
export const portableActions = new PendingActions(storage, async (action) => {
  if (
    action.command.kind === 'backup_create' ||
    action.command.kind === 'note_export' ||
    action.command.kind === 'source_export' ||
    action.command.kind === 'diagnostics_export'
  ) {
    const accepted = await call(action.command, 'job_accepted', action.identity);
    if (accepted.receipt.command_id !== action.id)
      throw new Error('任务回执不匹配 / Task receipt does not match.');
    return { kind: 'job', id: accepted.job_id };
  }
  const accepted = await call(action.command, 'bootstrap');
  if (accepted.command_id !== action.id)
    throw new Error('操作回执不匹配 / Operation receipt does not match.');
  return { kind: 'operation', id: accepted.operation_id };
});
export function usePortableActions() {
  const state = useSyncExternalStore(
    portableActions.subscribe,
    portableActions.getSnapshot,
    portableActions.getSnapshot,
  );
  useEffect(() => {
    void portableActions.load().catch(() => undefined);
  }, []);
  return state;
}

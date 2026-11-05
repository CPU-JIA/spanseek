import type { AutomationIntent, AutomationIntentStorage } from './automation-intents';

let database: Promise<IDBDatabase> | undefined;
function open() {
  return (database ??= new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('spanseek-automation-requests', 1);
    request.onupgradeneeded = () =>
      request.result.createObjectStore('requests', { keyPath: 'key' });
    request.onerror = request.onblocked = () => {
      database = undefined;
      reject(new Error('无法读取自动化请求记录 / Cannot open automation request storage'));
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
async function transact(
  key: string,
  update?: (current: AutomationIntent | undefined, store: IDBObjectStore) => void,
): Promise<AutomationIntent | undefined> {
  const db = await open();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(
      'requests',
      update ? 'readwrite' : 'readonly',
      update ? { durability: 'strict' } : undefined,
    );
    const store = tx.objectStore('requests');
    const request = store.get(key);
    let failure: unknown;
    request.onsuccess = () => {
      try {
        update?.(request.result, store);
      } catch (error) {
        failure = error;
        tx.abort();
      }
    };
    tx.oncomplete = () => resolve(request.result);
    tx.onerror = tx.onabort = () =>
      reject(
        failure ?? new Error('无法保存自动化请求记录 / Cannot save automation request record'),
      );
  });
}
const same = (a: AutomationIntent, b: AutomationIntent) => JSON.stringify(a) === JSON.stringify(b);
export const automationStorage: AutomationIntentStorage = {
  read: (key) => transact(key),
  claim: async (intent) => {
    await transact(intent.key, (current, store) => {
      if (current && !same(current, intent))
        throw new Error(
          '请先确认本知识库的原请求 / Confirm this workspace’s original request first',
        );
      if (!current) store.add(intent);
    });
  },
  confirm: async (intent) => {
    await transact(intent.key, (current, store) => {
      if (current && !same(current, intent))
        throw new Error('待确认记录已变化，未清除 / Pending record changed; it was not removed');
      if (current) store.delete(intent.key);
    });
  },
};

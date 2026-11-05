import { useEffect, useState, useSyncExternalStore } from 'react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { CreateActions, createKey } from './create-actions';
import { createStorage } from './create-storage';
const controllers = new Map<string, Promise<CreateActions>>();
const subscribeEmpty = () => () => undefined;
const empty = () => null;
export function useCreateNote(identity: WorkspaceIdentity | null) {
  const key = identity ? createKey(identity) : '';
  const [loaded, setLoaded] = useState<{ key: string; controller: CreateActions } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setError(null);
    if (!identity) return;
    let live = true;
    let pending = controllers.get(key);
    if (!pending) {
      pending = CreateActions.open(identity, createStorage, (command) =>
        call(command, 'note_commit', identity),
      );
      controllers.set(key, pending);
      void pending.catch(() => controllers.delete(key));
    }
    void pending.then(
      (controller) => {
        if (live) setLoaded({ key, controller });
      },
      (error) => {
        if (live) setError(message(error));
      },
    );
    return () => {
      live = false;
    };
  }, [key, attempt]);
  const controller = loaded?.key === key ? loaded.controller : null;
  const state = useSyncExternalStore(
    controller?.subscribe ?? subscribeEmpty,
    controller?.getSnapshot ?? empty,
  );
  return { controller, state, error, reload: () => setAttempt((value) => value + 1) };
}

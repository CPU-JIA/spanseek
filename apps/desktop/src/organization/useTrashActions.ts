import { useEffect, useState, useSyncExternalStore } from 'react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { deleteDraftRecords } from '../editor/drafts';
import { PageActions, pageKey } from './page-actions';
import { pageIntentStorage } from './page-intent-storage';
import { TrashActions, trashKey } from './trash-actions';
import { trashIntentStorage } from './trash-intent-storage';

const controllers = new Map<string, Promise<PageActions>>();
const lifecycleControllers = new Map<string, Promise<TrashActions>>();
const subscribeEmpty = () => () => undefined;
const empty = () => null;

/** Load the durable lifecycle intent before mounting any editor. */
export function useTrashActions(identity: WorkspaceIdentity | null) {
  const key = identity ? pageKey(identity, 'note-trash') : '';
  const [loaded, setLoaded] = useState<{ key: string; controller: PageActions } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setError(null);
    if (!identity) return;
    let live = true;
    let pending = controllers.get(key);
    if (!pending) {
      pending = PageActions.open(identity, 'note-trash', pageIntentStorage, (command) =>
        call(command, 'metadata_commit', identity),
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

/** Resolve durable permanent-delete/batch-restore intents before reopening editors. */
export function useTrashLifecycle(identity: WorkspaceIdentity | null) {
  const key = identity ? trashKey(identity) : '';
  const [loaded, setLoaded] = useState<{ key: string; controller: TrashActions } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setError(null);
    if (!identity) return;
    let live = true;
    let pending = lifecycleControllers.get(key);
    if (!pending) {
      pending = TrashActions.open(identity, trashIntentStorage, {
        delete: (command) => call(command, 'trash_commit', identity),
        restore: (command) => call(command, 'metadata_commit', identity),
        finalizeDelete: (commit) =>
          deleteDraftRecords(
            identity,
            commit.deleted_objects
              .filter((item) => item.kind === 'note')
              .map((item) => item.object_id),
          ),
      });
      lifecycleControllers.set(key, pending);
      void pending.catch(() => {
        if (lifecycleControllers.get(key) === pending) lifecycleControllers.delete(key);
      });
    }
    void pending.then(
      (controller) => {
        if (live) setLoaded({ key, controller });
      },
      (cause) => {
        if (live) setError(message(cause));
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

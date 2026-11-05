import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import {
  PageActions,
  pageKey,
  type OrganizationArea,
  type OrganizationCommand,
} from './page-actions';
import { pageIntentStorage } from './page-intent-storage';
import { ScopedQuery, type CursorPage } from './scoped-query';

const actions = new Map<string, Promise<PageActions>>();
async function send(command: OrganizationCommand, identity: WorkspaceIdentity) {
  if (command.kind.startsWith('collections_')) return call(command, 'collection_commit', identity);
  if (command.kind.startsWith('views_')) return call(command, 'view_commit', identity);
  if (command.kind.startsWith('canvases_')) return call(command, 'canvas_commit', identity);
  if (command.kind === 'notes_save') return call(command, 'note_commit', identity);
  return call(command, 'metadata_commit', identity);
}
export function usePageActions(identity: WorkspaceIdentity, area: OrganizationArea) {
  const key = pageKey(identity, area);
  const [loaded, setLoaded] = useState<{ key: string; value: PageActions } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let live = true;
    setError(null);
    let pending = actions.get(key);
    if (!pending) {
      pending = PageActions.open(identity, area, pageIntentStorage, (command) =>
        send(command, identity),
      );
      actions.set(key, pending);
      void pending.catch(() => {
        if (actions.get(key) === pending) actions.delete(key);
      });
    }
    void pending
      .then((value) => {
        if (live) setLoaded({ key, value });
      })
      .catch((error) => {
        if (live) setError(message(error));
      });
    return () => {
      live = false;
    };
  }, [key, attempt]);
  return {
    controller: loaded?.key === key ? loaded.value : null,
    error,
    retry: () => setAttempt((value) => value + 1),
  };
}
export function useCursorPage<T>(
  scope: string,
  read: (cursor: string | null) => Promise<CursorPage<T>>,
) {
  // Every scope includes identity and all query inputs. Old responses cannot cross instances.
  const query = useMemo(() => new ScopedQuery(read), [scope]);
  const state = useSyncExternalStore(query.subscribe, query.getSnapshot);
  useEffect(() => {
    void query.load();
    return () => query.invalidate();
  }, [query]);
  return { ...state, reload: () => query.load(), more: () => query.load(true) };
}

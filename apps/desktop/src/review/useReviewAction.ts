import { useEffect, useRef, useState } from 'react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call, message, ServiceError } from '../api';
import { invalidateReviewIndex } from './review-status';
import {
  confirmReviewIntent,
  storeReviewIntent,
  validateReviewIntent,
  type ReviewCommand,
  type ReviewIntent,
} from './review-intents';

/** Mounted inside identity-keyed page/button scopes. Exact requests survive reload. */
export function useReviewAction(identity: WorkspaceIdentity, scope?: string) {
  const key = JSON.stringify([
    'pkms-review-intent-v1',
    identity.local_instance_id,
    identity.workspace_id,
    ...(scope ? [scope] : []),
  ]);
  const [pending, setPending] = useState<ReviewIntent | null>(null);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const locked = useRef(false);
  useEffect(() => {
    alive.current = true;
    try {
      const raw = localStorage.getItem(key);
      const value: ReviewIntent | null = raw ? JSON.parse(raw) : null;
      if (value) validateReviewIntent(value, identity);
      setPending(value);
      setReady(true);
    } catch (error) {
      setError(message(error));
    }
    return () => {
      alive.current = false;
    };
  }, [key]);
  async function submit(command?: ReviewCommand) {
    if (!ready || locked.current || (command && pending)) return null;
    locked.current = true;
    setBusy(true);
    setError(null);
    let intent = pending;
    try {
      if (command) {
        intent = structuredClone({ identity, command });
        validateReviewIntent(intent, identity);
        storeReviewIntent(localStorage, key, intent);
        setPending(intent);
      }
      if (!intent) return null;
      validateReviewIntent(intent, identity);
      const result = await call(intent.command, 'review_mutation', intent.identity);
      if (
        result.receipt.command_id !== intent.command.payload.command_id ||
        (intent.command.kind !== 'review_add' &&
          result.entry.id !== intent.command.payload.request.entry_id)
      ) {
        throw new Error(
          '回顾回执不匹配，原请求已保留 / Review receipt mismatch; original request retained',
        );
      }
      confirmReviewIntent(localStorage, key, intent);
      // Every review mutation changes the shared queue read other controls show.
      invalidateReviewIndex(identity);
      if (alive.current) setPending(null);
      return result;
    } catch (error) {
      if (
        intent &&
        error instanceof ServiceError &&
        ['invalid_input', 'not_found', 'metadata_conflict', 'object_trashed'].includes(
          error.details.code,
        )
      ) {
        try {
          confirmReviewIntent(localStorage, key, intent);
          if (alive.current) setPending(null);
        } catch (storageError) {
          if (alive.current) setError(`${message(error)} · ${message(storageError)}`);
          return null;
        }
      }
      if (alive.current) setError(message(error));
      return null;
    } finally {
      locked.current = false;
      if (alive.current) setBusy(false);
    }
  }
  return { pending, busy, error, locked: !ready || busy || !!pending, submit };
}

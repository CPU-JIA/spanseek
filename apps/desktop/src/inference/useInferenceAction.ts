import { useEffect, useRef, useState } from 'react';
import type { DesktopCommand, DesktopData, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { inferenceErrorMessage } from './inference-errors';
import {
  validateInferenceIntent as validate,
  storeInferenceIntent,
  confirmInferenceIntent,
  type InferenceIntent as Intent,
} from './inference-intents';

type Kind = DesktopData['kind'];
type Result<K extends Kind> =
  Extract<DesktopData, { kind: K }> extends { payload: infer P } ? P : undefined;

/** Single main-window page scope; persist exact mutation before IPC and keep unknown outcomes. */
export function useInferenceAction(
  identity: WorkspaceIdentity,
  scope: string,
  formatError?: (error: unknown) => string,
) {
  const { t } = useI18n();
  const key = JSON.stringify([
    'pkms-inference-intent-v1',
    identity.local_instance_id,
    identity.workspace_id,
    scope,
  ]);
  const [pending, setPending] = useState<Intent | null>(null);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const locked = useRef(false);
  const live = useRef(true);
  function load() {
    try {
      const raw = localStorage.getItem(key);
      const intent: Intent | null = raw ? JSON.parse(raw) : null;
      if (intent) validate(intent, identity);
      setPending(intent);
      setReady(true);
      setError(null);
    } catch (e) {
      setError(message(e));
    }
  }
  useEffect(() => {
    live.current = true;
    load();
    return () => {
      live.current = false;
    };
  }, [key]);
  async function dispatch<K extends Kind>(intent: Intent): Promise<Result<K> | null> {
    if (locked.current) return null;
    locked.current = true;
    setBusy(true);
    setError(null);
    try {
      validate(intent, identity);
      const result = await call(intent.command, intent.expected, intent.identity);
      // A newer intent must never be removed by an older response.
      confirmInferenceIntent(localStorage, key, intent);
      if (live.current) setPending(null);
      return result as Result<K>;
    } catch (e) {
      const code = (e as { details?: { code?: string } })?.details?.code;
      if (
        code &&
        [
          'invalid_input',
          'invalid_document',
          'not_found',
          'metadata_conflict',
          'revision_conflict',
          'object_trashed',
          'query_expired',
        ].includes(code)
      ) {
        try {
          if (localStorage.getItem(key) === JSON.stringify(intent)) {
            localStorage.removeItem(key);
            if (live.current) setPending(null);
          }
        } catch (storageError) {
          if (live.current) setError(`${message(e)} · ${message(storageError)}`);
          return null;
        }
      }
      if (live.current) setError(formatError ? formatError(e) : inferenceErrorMessage(e, t));
      return null;
    } finally {
      locked.current = false;
      if (live.current) setBusy(false);
    }
  }
  async function execute<K extends Kind>(
    command: DesktopCommand,
    expected: K,
  ): Promise<Result<K> | null> {
    if (!ready || pending || locked.current) return null;
    const intent = structuredClone({ identity, command, expected });
    try {
      validate(intent, identity);
      storeInferenceIntent(localStorage, key, intent);
      setPending(intent);
    } catch (e) {
      setError(message(e));
      return null;
    }
    return dispatch<K>(intent);
  }
  async function retry<K extends Kind>(): Promise<Result<K> | null> {
    if (!pending) {
      load();
      return null;
    }
    return dispatch<K>(pending);
  }
  return {
    pending,
    ready,
    busy,
    error,
    execute,
    retry,
    clearError: () => setError(null),
    locked: !ready || busy || !!pending,
  };
}

import { useEffect, useRef, useState } from 'react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { message } from '../api';
import {
  AutomationActions,
  automationIntentKey,
  type AutomationCommand,
  type AutomationOutcome,
} from './automation-intents';
import { automationStorage } from './automation-storage';
import { sendAutomationCommand } from './automation-transport';

const controllers = new Map<string, Promise<AutomationActions>>();
function get(identity: WorkspaceIdentity) {
  const key = automationIntentKey(identity);
  let promise = controllers.get(key);
  if (!promise) {
    promise = AutomationActions.open(identity, automationStorage, (command) =>
      sendAutomationCommand(command, identity),
    );
    controllers.set(key, promise);
    void promise.catch(() => controllers.delete(key));
  }
  return promise;
}
/** Identity-keyed page scopes share in-flight requests across unmounts. */
export function useAutomationActions(
  identity: WorkspaceIdentity,
  navigationKey: string,
  consume: (outcome: AutomationOutcome) => void,
) {
  const [controller, setController] = useState<AutomationActions | null>(null);
  const [snapshot, setSnapshot] = useState<ReturnType<AutomationActions['getSnapshot']> | null>(
    null,
  );
  const [loadError, setLoadError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const alive = useRef(false);
  const consumer = useRef(consume);
  consumer.current = consume;
  const navigation = useRef(navigationKey);
  navigation.current = navigationKey;
  const key = automationIntentKey(identity);
  useEffect(() => {
    alive.current = true;
    let live = true;
    let unsubscribe: (() => void) | undefined;
    setLoadError(null);
    void get(identity)
      .then((value) => {
        if (!live) return;
        setController(value);
        setSnapshot(value.getSnapshot());
        unsubscribe = value.subscribe(() => setSnapshot(value.getSnapshot()));
      })
      .catch((error) => {
        if (live) setLoadError(message(error));
      });
    return () => {
      live = false;
      alive.current = false;
      unsubscribe?.();
    };
  }, [key, attempt]);
  const submit = (command?: AutomationCommand) => {
    const origin = navigation.current;
    return controller?.run(command, (outcome) => {
      // A late reply cannot replace a destination selected while it was running.
      // Keep the exact request until the user explicitly confirms it there.
      if (!alive.current || navigation.current !== origin) return false;
      consumer.current(outcome);
      return true;
    });
  };
  return {
    pending: snapshot?.pending ?? null,
    busy: snapshot?.busy ?? false,
    ready: !!controller,
    error: loadError ?? snapshot?.error ?? null,
    locked: !controller || !!snapshot?.busy || !!snapshot?.pending,
    submit,
    reload: () => setAttempt((value) => value + 1),
  };
}

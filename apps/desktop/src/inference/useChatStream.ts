import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { isTauri } from '@tauri-apps/api/core';
import type { ChatProgress, ChatRun, DesktopEvent, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import type { ChatStreamSeed } from './chat-history';
import { latestChatProgress } from './chat-progress';
import { appendChatEvents, chatActive, emptyChatStream } from './chat-stream';

export function useChatStream(
  identity: WorkspaceIdentity,
  runId: string | null,
  seed: ChatStreamSeed | null = null,
) {
  const [stream, setStream] = useState(emptyChatStream);
  const [progress, setProgress] = useState<ChatProgress | null>(null);
  const [run, setRun] = useState<ChatRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const handleEvent = useRef<((event: DesktopEvent) => void) | null>(null);
  // Changing conversations only changes the JS handler. Repeated native unlisten/listen
  // while events are queued can unregister a callback before its last delivery arrives.
  useEffect(() => {
    if (!isTauri()) return;
    let live = true;
    const subscription = listen<DesktopEvent>('pkms:event', ({ payload }) => {
      if (live) handleEvent.current?.(payload);
    });
    void subscription.catch((e) => {
      if (live) setError(message(e));
    });
    return () => {
      live = false;
      void subscription.then(
        (unlisten) => unlisten(),
        () => {},
      );
    };
  }, [identity.local_instance_id, identity.workspace_id]);
  useEffect(() => {
    let live = true;
    let busy = false;
    let requested = false;
    const initial = seed?.run.id === runId ? seed : null;
    let current = initial?.stream ?? emptyChatStream();
    setStream(current);
    setRun(initial?.run ?? null);
    setProgress(null);
    setError(null);
    if (!runId) return;
    async function refresh() {
      if (!live) return;
      if (busy) {
        requested = true;
        return;
      }
      busy = true;
      try {
        do {
          requested = false;
          let more = true;
          while (live && more) {
            const events = await call(
              {
                kind: 'chat_runs_events',
                payload: {
                  run_id: runId!,
                  after_seq: current.seq,
                  limit: 100,
                },
              },
              'chat_run_events',
              identity,
            );
            if (!live) return;
            current = appendChatEvents(current, events);
            setStream(current);
            more = events.length === 100;
          }
          const latest = await call(
            { kind: 'chat_runs_get', payload: { run_id: runId! } },
            'chat_run',
            identity,
          );
          if (!live) return;
          setRun(latest);
          // Metadata may be newer than the last event page. Drain before treating it as settled.
          requested ||= latest.last_event_seq > current.seq;
          setError(null);
        } while (live && requested);
      } catch (e) {
        if (live) setError(message(e));
      } finally {
        busy = false;
      }
    }
    // Completed snapshots already include every persisted delta; do not replay their full log.
    if (!initial || chatActive(current.status)) void refresh();
    const timer = window.setInterval(() => {
      if (chatActive(current.status)) void refresh();
    }, 1000);
    handleEvent.current = (payload) => {
      if (!live) return;
      if (payload.kind === 'chat_progress') {
        setProgress((previous) =>
          latestChatProgress(
            previous,
            payload.payload.progress,
            identity,
            payload.payload.identity,
            runId,
            current.status,
          ),
        );
        return;
      }
      if (
        payload.kind === 'chat_changed' &&
        payload.payload.run_id === runId &&
        payload.payload.identity.local_instance_id === identity.local_instance_id &&
        payload.payload.identity.workspace_id === identity.workspace_id
      )
        void refresh();
    };
    return () => {
      live = false;
      handleEvent.current = null;
      window.clearInterval(timer);
    };
  }, [identity.local_instance_id, identity.workspace_id, runId, seed]);
  return { stream, run, progress, error };
}

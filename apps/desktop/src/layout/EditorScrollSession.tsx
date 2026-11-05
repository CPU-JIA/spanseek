import { useLayoutEffect, useRef, type ReactNode } from 'react';
import type { SessionPositions } from './session-positions';

/** Owns only the existing editor scroller; source readers retain their separate reading-state contract. */
export function EditorScrollSession({
  positions,
  pane,
  noteId,
  referenceRequest,
  children,
}: {
  positions: SessionPositions;
  pane: number;
  noteId: string;
  referenceRequest?: string;
  children: ReactNode;
}) {
  const root = useRef<HTMLDivElement>(null);
  const explicitNavigation = useRef(!!referenceRequest);
  const stopRestore = useRef<() => void>(() => undefined);
  explicitNavigation.current = !!referenceRequest;
  useLayoutEffect(() => {
    if (referenceRequest) stopRestore.current();
  }, [referenceRequest]);
  useLayoutEffect(() => {
    const host = root.current;
    if (!host) return;
    let scroller: HTMLElement | null = null;
    let resize: ResizeObserver | undefined;
    let restoring = !explicitNavigation.current;
    const target = positions.scroll(pane, noteId) ?? { x: 0, y: 0 };
    const capture = () => {
      if (scroller?.isConnected && !restoring)
        positions.recordScroll(pane, noteId, scroller.scrollLeft, scroller.scrollTop);
    };
    const userIntent = () => {
      restoring = false;
    };
    stopRestore.current = userIntent;
    const restore = () => {
      if (!scroller || !restoring || explicitNavigation.current) return;
      // A lazy editor/image may not have its final height yet. ResizeObserver retries
      // this target, without writing a clamped zero back over the saved position.
      scroller.scrollLeft = target.x;
      scroller.scrollTop = target.y;
    };
    const bind = () => {
      if (scroller) {
        restore();
        return;
      }
      const found = host.querySelector<HTMLElement>('.document-scroll');
      if (!found?.querySelector('.note-prose')) return;
      scroller = found;
      scroller.addEventListener('scroll', capture, { passive: true });
      scroller.addEventListener('wheel', userIntent, { passive: true });
      scroller.addEventListener('touchstart', userIntent, { passive: true });
      scroller.addEventListener('pointerdown', userIntent, { passive: true });
      scroller.addEventListener('keydown', userIntent);
      resize = new ResizeObserver(restore);
      resize.observe(scroller);
      const content = scroller.querySelector('.document-content');
      if (content) resize.observe(content);
      restore();
    };
    const mutation = new MutationObserver(bind);
    mutation.observe(host, { childList: true, subtree: true });
    bind();
    const unregister = positions.registerCapture(capture);
    return () => {
      capture();
      unregister();
      mutation.disconnect();
      resize?.disconnect();
      scroller?.removeEventListener('scroll', capture);
      scroller?.removeEventListener('wheel', userIntent);
      scroller?.removeEventListener('touchstart', userIntent);
      scroller?.removeEventListener('pointerdown', userIntent);
      scroller?.removeEventListener('keydown', userIntent);
      stopRestore.current = () => undefined;
    };
  }, [positions, pane, noteId]);
  // display:contents preserves the EditorPane fragments' existing flex layout.
  return (
    <div ref={root} style={{ display: 'contents' }}>
      {children}
    </div>
  );
}

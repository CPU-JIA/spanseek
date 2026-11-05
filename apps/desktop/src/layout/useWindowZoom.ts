import { useEffect, useRef } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { WindowZoom } from './window-zoom';

/**
 * Applies the window zoom and keeps the stored preference in step with it.
 *
 * The zoom belongs to the window, not to a note: it is a view scale, never a
 * font-size override, and it never changes stored formatting. `storedPercent`
 * is the saved preference (`zoom_percent`); `onScaleChange` reports every scale
 * the user reaches with the wheel or the keyboard so it can be saved.
 */
export function useWindowZoom({
  storedPercent,
  onError,
  onScaleChange,
}: {
  storedPercent: number | null;
  onError: (error: unknown) => void;
  onScaleChange: (percent: number) => void;
}) {
  const report = useRef(onError);
  report.current = onError;
  const remember = useRef(onScaleChange);
  remember.current = onScaleChange;
  const zoom = useRef<WindowZoom | null>(null);
  /** The scale already applied, so a stored value equal to it needs no write. */
  const applied = useRef<number | null>(null);
  const stored = useRef(storedPercent);
  stored.current = storedPercent;

  useEffect(() => {
    if (!isTauri()) return;
    let live = true;
    // WebView2 applies zoom without resizing the observed application element
    // and the first zoom runs before the layout listener mounts, so sample the
    // layout viewport a few times after every zoom write.
    const announceLayout = () => {
      for (const delay of [0, 40, 120, 300]) {
        window.setTimeout(() => window.dispatchEvent(new Event('resize')), delay);
      }
    };
    const controller = new WindowZoom(async (scale) => {
      await getCurrentWebview().setZoom(scale);
      announceLayout();
    });
    zoom.current = controller;
    const initial = (stored.current ?? 100) / 100;
    applied.current = initial;
    // A reload starts at a known native scale, so the stored value is applied
    // rather than assumed.
    void controller.reset(initial).catch((error) => {
      if (live) report.current(error);
    });
    const run = (operation: Promise<void>) => {
      void operation
        .then(() => {
          if (!live) return;
          const scale = controller.current();
          applied.current = scale;
          remember.current(Math.round(scale * 100));
        })
        .catch((error) => {
          if (live) report.current(error);
        });
    };
    const wheel = (event: WheelEvent) => {
      if (
        event.defaultPrevented ||
        !event.ctrlKey ||
        event.altKey ||
        event.metaKey ||
        (event.target instanceof Element && event.target.closest('[data-local-zoom="pdf"]'))
      )
        return;
      event.preventDefault();
      run(controller.wheel(event.deltaY, event.deltaMode));
    };
    const keydown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        (event.target instanceof Element && event.target.closest('[data-local-zoom="pdf"]')) ||
        event.isComposing ||
        event.keyCode === 229 ||
        !(event.ctrlKey || event.metaKey) ||
        event.altKey
      )
        return;
      if (!['+', '=', '-', '0'].includes(event.key)) return;
      event.preventDefault();
      run(event.key === '0' ? controller.reset() : controller.step(event.key === '-' ? -1 : 1));
    };
    window.addEventListener('wheel', wheel, { passive: false });
    window.addEventListener('keydown', keydown);
    return () => {
      live = false;
      zoom.current = null;
      window.removeEventListener('wheel', wheel);
      window.removeEventListener('keydown', keydown);
    };
  }, []);

  // The stored preference arrives after the first paint. Apply it once, but
  // never re-apply the value this hook just reported: that would fight the user.
  useEffect(() => {
    const controller = zoom.current;
    if (!controller || storedPercent === null) return;
    const scale = storedPercent / 100;
    if (applied.current !== null && Math.abs(applied.current - scale) < 0.0005) return;
    applied.current = scale;
    void controller.reset(scale).catch((error) => report.current(error));
  }, [storedPercent]);
}

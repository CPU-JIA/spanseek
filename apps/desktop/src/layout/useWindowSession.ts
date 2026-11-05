import { useCallback, useEffect, useRef } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { WindowGeometry } from '@pkms/contracts';
import {
  availableMonitors,
  getCurrentWindow,
  PhysicalPosition,
  PhysicalSize,
} from '@tauri-apps/api/window';
import { WindowSession } from './window-session';
import type { SessionPositions } from './session-positions';

export function useWindowSession(
  positions: SessionPositions | null,
  onError: (error: unknown) => void,
) {
  const report = useRef(onError);
  report.current = onError;
  const native = useRef<WindowSession | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    if (!isTauri()) return;
    const window = getCurrentWindow();
    const controller = new WindowSession({
      read: async () => {
        const [size, outer, position, maximized, minimized, fullscreen, scale, restoreGeometry] =
          await Promise.all([
            window.innerSize(),
            window.outerSize(),
            window.outerPosition(),
            window.isMaximized(),
            window.isMinimized(),
            window.isFullscreen(),
            window.scaleFactor(),
            invoke<WindowGeometry | null>('read_window_restore_geometry'),
          ]);
        return {
          width: size.width,
          height: size.height,
          left: position.x,
          top: position.y,
          maximized,
          minimized,
          fullscreen,
          frameWidth: outer.width - size.width,
          frameHeight: outer.height - size.height,
          scale,
          restoreGeometry,
        };
      },
      monitors: async () =>
        (await availableMonitors()).map((monitor) => ({
          left: monitor.workArea.position.x,
          top: monitor.workArea.position.y,
          width: monitor.workArea.size.width,
          height: monitor.workArea.size.height,
          scale: monitor.scaleFactor,
        })),
      maximize: (maximized) => (maximized ? window.maximize() : window.unmaximize()),
      position: (left, top) => window.setPosition(new PhysicalPosition(left, top)),
      size: (width, height) => window.setSize(new PhysicalSize(width, height)),
    });
    native.current = controller;
    let live = true;
    const changed = () => {
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        if (live) void controller.sample().catch((error) => report.current(error));
      }, 150);
    };
    const listeners = [
      window.onMoved(changed),
      window.onResized(changed),
      window.onScaleChanged(changed),
    ];
    for (const listener of listeners)
      void listener.catch((error) => {
        if (live) report.current(error);
      });
    return () => {
      live = false;
      clearTimeout(timer.current);
      void controller.bind(null).catch((error) => report.current(error));
      if (native.current === controller) native.current = null;
      for (const listener of listeners)
        void listener.then(
          (unlisten) => unlisten(),
          () => undefined,
        );
    };
  }, []);
  useEffect(() => {
    clearTimeout(timer.current);
    void native.current?.bind(positions).catch((error) => report.current(error));
  }, [positions]);
  return useCallback(async () => {
    clearTimeout(timer.current);
    // Retry a failed current restore before allowing the close/switch barrier to save.
    await native.current?.captureCurrent();
  }, []);
}

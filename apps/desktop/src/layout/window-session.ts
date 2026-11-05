import type { WindowGeometry } from '@pkms/contracts';
import type { SessionPositions } from './session-positions';

export type MonitorArea = {
  left: number;
  top: number;
  width: number;
  height: number;
  scale: number;
};
export type WindowSnapshot = WindowGeometry & {
  restoreGeometry?: WindowGeometry | null;
  minimized: boolean;
  fullscreen: boolean;
  frameWidth: number;
  frameHeight: number;
  scale: number;
};
export type WindowPort = {
  read: () => Promise<WindowSnapshot>;
  monitors: () => Promise<MonitorArea[]>;
  maximize: (maximized: boolean) => Promise<void>;
  position: (left: number, top: number) => Promise<void>;
  size: (width: number, height: number) => Promise<void>;
};

/** Persisted values are physical client size and physical outer position, including negative monitors. */
export function fitWindowGeometry(
  saved: WindowGeometry,
  monitors: MonitorArea[],
  frame: Pick<WindowSnapshot, 'frameWidth' | 'frameHeight' | 'scale'>,
): WindowGeometry {
  if (!monitors.length)
    throw new Error('无法读取显示器工作区，原窗口位置已保留。 / Monitor work area is unavailable.');
  const overlap = (monitor: MonitorArea) =>
    Math.max(
      0,
      Math.min(saved.left + saved.width, monitor.left + monitor.width) -
        Math.max(saved.left, monitor.left),
    ) *
    Math.max(
      0,
      Math.min(saved.top + saved.height, monitor.top + monitor.height) -
        Math.max(saved.top, monitor.top),
    );
  const distance = (monitor: MonitorArea) =>
    Math.hypot(
      saved.left + saved.width / 2 - (monitor.left + monitor.width / 2),
      saved.top + saved.height / 2 - (monitor.top + monitor.height / 2),
    );
  const monitor = [...monitors].sort(
    (a, b) => overlap(b) - overlap(a) || distance(a) - distance(b),
  )[0];
  const scale = monitor.scale / frame.scale;
  const borderWidth = Math.max(0, Math.round(frame.frameWidth * scale));
  const borderHeight = Math.max(0, Math.round(frame.frameHeight * scale));
  // Match tauri.conf.json's logical minimums; a smaller monitor still keeps the title bar reachable.
  const minWidth = Math.ceil(720 * monitor.scale),
    minHeight = Math.ceil(540 * monitor.scale);
  const width = Math.max(minWidth, Math.min(saved.width, monitor.width - borderWidth));
  const height = Math.max(minHeight, Math.min(saved.height, monitor.height - borderHeight));
  return {
    ...saved,
    width,
    height,
    left: Math.round(
      Math.max(
        monitor.left,
        Math.min(saved.left, monitor.left + Math.max(0, monitor.width - width - borderWidth)),
      ),
    ),
    top: Math.round(
      Math.max(
        monitor.top,
        Math.min(saved.top, monitor.top + Math.max(0, monitor.height - height - borderHeight)),
      ),
    ),
  };
}

/** Serialize native reads/restores. A superseded workspace cannot publish or finish restoring over a newer one. */
export class WindowSession {
  private generation = 0;
  private active: SessionPositions | null = null;
  private normal: WindowGeometry | null = null;
  private ready = false;
  private operations: Promise<void> = Promise.resolve();
  private port: WindowPort;
  constructor(port: WindowPort) {
    this.port = port;
  }

  private serial(operation: () => Promise<void>) {
    const next = this.operations.then(operation, operation);
    this.operations = next;
    return next;
  }

  bind(positions: SessionPositions | null): Promise<void> {
    const generation = ++this.generation;
    this.active = positions;
    this.ready = false;
    const current = () => generation === this.generation && this.active === positions;
    return this.serial(async () => {
      if (!current()) return;
      const snapshot = await this.port.read();
      if (!current()) return;
      if (!snapshot.maximized && !snapshot.minimized && !snapshot.fullscreen)
        this.normal = this.geometry(snapshot);
      const saved = positions?.queue.getSnapshot().session.layout.window;
      if (!saved) {
        this.ready = true;
        // The home screen has no saved workspace rectangle, so the configured
        // default window is kept inside the monitor work area instead of
        // overflowing a smaller display.
        const fitted = await this.fitHomeScreen(snapshot, current);
        if (!current()) return;
        this.capture(fitted ? { ...snapshot, ...fitted } : snapshot, positions);
        return;
      }
      const monitors = await this.port.monitors();
      if (!current()) return;
      // No position event can enqueue a capture ahead of this complete restore.
      if (snapshot.maximized) {
        await this.port.maximize(false);
        if (!current()) return;
      }
      const normalSnapshot = snapshot.maximized ? await this.port.read() : snapshot;
      if (!current()) return;
      const fitted = fitWindowGeometry(saved, monitors, normalSnapshot);
      await this.port.position(fitted.left, fitted.top);
      if (!current()) return;
      await this.port.size(fitted.width, fitted.height);
      if (!current()) return;
      // Moving across DPI boundaries can itself resize/reposition the native window.
      await this.port.position(fitted.left, fitted.top);
      if (!current()) return;
      this.normal = { ...fitted, maximized: false };
      if (fitted.maximized) {
        await this.port.maximize(true);
        if (!current()) return;
      }
      const actual = await this.port.read();
      if (current()) {
        this.ready = true;
        this.capture(actual, positions);
      }
    });
  }

  sample(): Promise<void> {
    const generation = this.generation,
      positions = this.active;
    return this.serial(async () => {
      if (generation !== this.generation || !this.ready) return;
      const snapshot = await this.port.read();
      if (generation === this.generation && this.active === positions)
        this.capture(snapshot, positions);
    });
  }

  async captureCurrent(): Promise<void> {
    try {
      await this.operations;
    } catch {
      // The earlier operation reported its failure. An explicit close/switch may
      // retry it; new errors below still reject the caller's persistence barrier.
    }
    if (!this.ready) await this.bind(this.active);
    await this.sample();
  }

  /** Fits the initial home-screen window into the work area; returns the applied rectangle. */
  private async fitHomeScreen(snapshot: WindowSnapshot, current: () => boolean) {
    if (snapshot.maximized || snapshot.minimized || snapshot.fullscreen) return null;
    const monitors = await this.port.monitors();
    if (!current() || !monitors.length) return null;
    const fitted = fitWindowGeometry(this.geometry(snapshot), monitors, snapshot);
    if (
      fitted.width === snapshot.width &&
      fitted.height === snapshot.height &&
      fitted.left === snapshot.left &&
      fitted.top === snapshot.top
    )
      return null;
    await this.port.position(fitted.left, fitted.top);
    if (!current()) return null;
    await this.port.size(fitted.width, fitted.height);
    if (!current()) return null;
    await this.port.position(fitted.left, fitted.top);
    return fitted;
  }

  private geometry(snapshot: WindowGeometry): WindowGeometry {
    return {
      width: snapshot.width,
      height: snapshot.height,
      left: snapshot.left,
      top: snapshot.top,
      maximized: snapshot.maximized,
    };
  }

  private capture(snapshot: WindowSnapshot, positions: SessionPositions | null) {
    if (snapshot.minimized || snapshot.fullscreen) return;
    if (snapshot.restoreGeometry) this.normal = this.geometry(snapshot.restoreGeometry);
    else if (!snapshot.maximized) this.normal = this.geometry(snapshot);
    // On the home screen, maintain native restore bounds without writing any workspace.
    if (!this.normal || !positions) return;
    positions.recordWindow({
      ...this.normal,
      maximized: snapshot.restoreGeometry?.maximized ?? snapshot.maximized,
    });
  }
}

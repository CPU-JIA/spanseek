import type { ScrollOffset, WindowGeometry } from '@pkms/contracts';
import type { SessionQueue, LayoutChange } from './session-queue';

/** Position changes share the layout queue; they never replace tabs, drafts or unknown fields. */
export class SessionPositions {
  private offsets = new Map<string, { pane: number; value: ScrollOffset }>();
  private dirty = new Set<string>();
  private window: WindowGeometry | null = null;
  private windowDirty = false;
  private captures = new Set<() => void>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;
  readonly queue: SessionQueue;
  private onError: (error: unknown) => void;

  constructor(queue: SessionQueue, onError: (error: unknown) => void) {
    this.queue = queue;
    this.onError = onError;
  }

  scroll(pane: number, objectId: string): ScrollOffset | undefined {
    return (
      this.offsets.get(`${pane}/${objectId}`)?.value ??
      this.queue
        .getSnapshot()
        .session.layout.panes[pane]?.scroll_offsets.find((offset) => offset.object_id === objectId)
    );
  }

  recordScroll(pane: number, objectId: string, x: number, y: number) {
    if (this.disposed || !Number.isFinite(x) || !Number.isFinite(y)) return;
    const value = {
      object_id: objectId,
      x: Math.max(0, Math.min(10_000_000, Math.round(x))),
      y: Math.max(0, Math.min(10_000_000, Math.round(y))),
    };
    const old = this.scroll(pane, objectId);
    if ((old?.x ?? 0) === value.x && (old?.y ?? 0) === value.y) return;
    const key = `${pane}/${objectId}`;
    this.offsets.set(key, { pane, value });
    this.dirty.add(key);
    this.schedule();
  }

  recordWindow(value: WindowGeometry) {
    if (this.disposed) return;
    const old = this.window ?? this.queue.getSnapshot().session.layout.window;
    if (
      old &&
      Object.keys(value).every(
        (key) => old[key as keyof WindowGeometry] === value[key as keyof WindowGeometry],
      )
    )
      return;
    this.window = { ...value };
    this.windowDirty = true;
    this.schedule();
  }

  registerCapture(capture: () => void) {
    this.captures.add(capture);
    return () => {
      this.captures.delete(capture);
    };
  }

  private schedule() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      void this.flush().catch(this.onError);
    }, 350);
  }

  async flush() {
    for (const capture of this.captures) capture();
    clearTimeout(this.timer);
    this.timer = undefined;
    const offsets = [...this.dirty].map((key) => this.offsets.get(key)!);
    const geometry = this.windowDirty && this.window ? { ...this.window } : null;
    this.dirty.clear();
    this.windowDirty = false;
    if (offsets.length || geometry) {
      const change: LayoutChange = (layout) => ({
        ...layout,
        ...(geometry ? { window: { ...layout.window, ...geometry } } : {}),
        panes: layout.panes.map((pane, index) => {
          const current = offsets.filter(
            (offset) =>
              offset.pane === index &&
              pane.tabs.some((tab) => tab.object_id === offset.value.object_id),
          );
          if (!current.length) return pane;
          return {
            ...pane,
            scroll_offsets: [
              ...pane.scroll_offsets.filter(
                (old) => !current.some(({ value }) => value.object_id === old.object_id),
              ),
              ...current.map(({ value }) => ({
                ...pane.scroll_offsets.find((old) => old.object_id === value.object_id),
                ...value,
              })),
            ],
          };
        }),
      });
      // Failed writes remain at the head of SessionQueue, including unknown receipts.
      await this.queue.enqueue(change);
    }
    await this.queue.flush();
  }

  dispose() {
    this.disposed = true;
    clearTimeout(this.timer);
    this.captures.clear();
  }
}

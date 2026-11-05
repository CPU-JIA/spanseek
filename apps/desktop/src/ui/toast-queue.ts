/**
 * Shell-level transient messages.
 *
 * Only instantaneous success/information copy belongs here. Blocking errors,
 * save conflicts and unconfirmed writes stay inline on their page with
 * `role="alert"`, because a message that vanishes after four seconds cannot
 * carry a retry.
 *
 * The queue owns ordering, de-duplication and lifetimes; the host owns pixels.
 * Timers are injected so tests advance a fake clock instead of sleeping, and
 * every list is replaced instead of mutated so React snapshots stay stable.
 */

export const TOAST_LIMIT = 3;
export const TOAST_LIFETIME_MS = 4_000;

export type ToastTone = 'success' | 'info';

export type ToastItem = {
  id: string;
  text: string;
  tone: ToastTone;
  /** Full lifetime in ms; the countdown starts when the item becomes visible. */
  lifetime: number;
  /** Whether the host renders its optional close button for this message. */
  dismissible: boolean;
};

export type ToastInput = {
  text: string;
  tone?: ToastTone;
  /** Overrides the queue default for this message only. */
  lifetime?: number;
  dismissible?: boolean;
  /** Call sites that already own an id (an undo action, a receipt) can keep it. */
  id?: string;
};

export type ToastSnapshot = {
  visible: readonly ToastItem[];
  queued: readonly ToastItem[];
};

export type ToastTimerHandle = ReturnType<typeof setTimeout>;

export type ToastQueueOptions = {
  /** How many messages stack at once; the rest wait in order. */
  limit?: number;
  lifetime?: number;
  now?: () => number;
  schedule?: (run: () => void, delay: number) => ToastTimerHandle;
  cancel?: (handle: ToastTimerHandle) => void;
  createId?: () => string;
};

/** Copy is compared after collapsing whitespace, so identical text is one toast. */
export function toastKey(text: string): string {
  return text.replace(/\s+/g, ' ').trim();
}

/** Reads CSS duration tokens such as `4000ms` or `4s`; anything else is ignored. */
export function parseDuration(value: string | null | undefined): number | null {
  if (!value) return null;
  const match = /^(\d+(?:\.\d+)?)(ms|s)$/.exec(value.trim());
  if (!match) return null;
  const amount = Number(match[1]);
  if (!Number.isFinite(amount)) return null;
  return Math.max(0, Math.round(match[2] === 's' ? amount * 1000 : amount));
}

function normalizeLifetime(value: number | undefined, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? Math.round(value)
    : fallback;
}

let toastSequence = 0;
function defaultToastId(): string {
  toastSequence += 1;
  return `toast-${toastSequence}`;
}

export class ToastQueue {
  private readonly limit: number;
  private readonly now: () => number;
  private readonly scheduleTimer: (run: () => void, delay: number) => ToastTimerHandle;
  private readonly cancelTimer: (handle: ToastTimerHandle) => void;
  private readonly createId: () => string;
  private readonly listeners = new Set<() => void>();
  private defaultLifetime: number;
  private visible: ToastItem[] = [];
  private queued: ToastItem[] = [];
  /** Running countdown per visible id. */
  private readonly handles = new Map<string, ToastTimerHandle>();
  private readonly deadlines = new Map<string, number>();
  /** Remaining ms per paused id; a paused toast never expires. */
  private readonly remaining = new Map<string, number>();
  /** True while the pointer or focus rests inside the host. */
  private held = false;
  private snapshot: ToastSnapshot = { visible: [], queued: [] };

  constructor(options: ToastQueueOptions = {}) {
    const limit = options.limit ?? TOAST_LIMIT;
    this.limit = Number.isFinite(limit) && limit >= 1 ? Math.floor(limit) : TOAST_LIMIT;
    this.defaultLifetime = normalizeLifetime(options.lifetime, TOAST_LIFETIME_MS);
    this.now = options.now ?? (() => Date.now());
    this.scheduleTimer = options.schedule ?? ((run, delay) => setTimeout(run, delay));
    this.cancelTimer = options.cancel ?? ((handle) => clearTimeout(handle));
    this.createId = options.createId ?? defaultToastId;
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getSnapshot = (): ToastSnapshot => this.snapshot;

  /** Returns the toast id, or null when blank copy is ignored. */
  notify(input: ToastInput | string): string | null {
    const text = toastKey(typeof input === 'string' ? input : (input.text ?? ''));
    if (!text) return null;
    const duplicate = this.findText(text);
    if (duplicate) {
      this.restart(duplicate.id);
      return duplicate.id;
    }
    const tone = typeof input === 'string' ? 'info' : (input.tone ?? 'info');
    const id = this.uniqueId(typeof input === 'string' ? undefined : input.id);
    const item: ToastItem = {
      id,
      text,
      tone,
      lifetime: normalizeLifetime(
        typeof input === 'string' ? undefined : input.lifetime,
        this.defaultLifetime,
      ),
      dismissible: typeof input === 'string' ? true : input.dismissible !== false,
    };
    if (this.visible.length < this.limit) {
      this.visible = [...this.visible, item];
      this.start(item.id, item.lifetime);
    } else {
      this.queued = [...this.queued, item];
    }
    this.emit();
    return id;
  }

  dismiss(id: string): void {
    if (!this.find(id)) return;
    this.stop(id);
    this.visible = this.visible.filter((item) => item.id !== id);
    this.queued = this.queued.filter((item) => item.id !== id);
    this.fill();
    this.emit();
  }

  /** Freezes one countdown; the remaining time is kept for `resume`. */
  pause(id: string): void {
    if (this.remaining.has(id)) return;
    const deadline = this.deadlines.get(id);
    if (deadline === undefined) return;
    this.clearHandle(id);
    this.deadlines.delete(id);
    this.remaining.set(id, Math.max(0, deadline - this.now()));
  }

  resume(id: string): void {
    const left = this.remaining.get(id);
    if (left === undefined) return;
    this.remaining.delete(id);
    if (!this.visible.some((item) => item.id === id)) return;
    this.deadlines.set(id, this.now() + left);
    this.handles.set(
      id,
      this.scheduleTimer(() => this.dismiss(id), left),
    );
  }

  /** Hovering or focusing the host freezes every countdown, including new ones. */
  pauseAll(): void {
    this.held = true;
    for (const item of this.visible) this.pause(item.id);
  }

  resumeAll(): void {
    this.held = false;
    for (const item of this.visible) this.resume(item.id);
  }

  clear(): void {
    for (const item of [...this.visible, ...this.queued]) this.stop(item.id);
    this.visible = [];
    this.queued = [];
    this.emit();
  }

  /** The host syncs this from the `--toast-lifetime` token; running toasts keep their own. */
  setDefaultLifetime(value: number): void {
    this.defaultLifetime = normalizeLifetime(value, this.defaultLifetime);
  }

  private find(id: string): ToastItem | undefined {
    return (
      this.visible.find((item) => item.id === id) ?? this.queued.find((item) => item.id === id)
    );
  }

  private findText(text: string): ToastItem | undefined {
    return (
      this.visible.find((item) => item.text === text) ??
      this.queued.find((item) => item.text === text)
    );
  }

  private uniqueId(requested: string | undefined): string {
    const trimmed = requested?.trim();
    if (trimmed && !this.find(trimmed)) return trimmed;
    let id = this.createId();
    while (this.find(id)) id = this.createId();
    return id;
  }

  /** A repeated message keeps its place and gets a full countdown again. */
  private restart(id: string): void {
    const item = this.visible.find((entry) => entry.id === id);
    if (!item) return;
    this.start(id, item.lifetime);
  }

  private start(id: string, delay: number): void {
    this.stop(id);
    if (this.held) {
      this.remaining.set(id, delay);
      return;
    }
    this.deadlines.set(id, this.now() + delay);
    this.handles.set(
      id,
      this.scheduleTimer(() => this.dismiss(id), delay),
    );
  }

  private stop(id: string): void {
    this.clearHandle(id);
    this.deadlines.delete(id);
    this.remaining.delete(id);
  }

  private clearHandle(id: string): void {
    const handle = this.handles.get(id);
    if (handle === undefined) return;
    this.cancelTimer(handle);
    this.handles.delete(id);
  }

  /** The queue drains in order, so a dismissed toast reveals the next message. */
  private fill(): void {
    while (this.visible.length < this.limit && this.queued.length > 0) {
      const [next, ...rest] = this.queued;
      this.queued = rest;
      this.visible = [...this.visible, next];
      this.start(next.id, next.lifetime);
    }
  }

  private emit(): void {
    this.snapshot = { visible: this.visible, queued: this.queued };
    for (const listener of this.listeners) listener();
  }
}

/** Shell singleton: `ToastHost` renders it and any call site pushes into it. */
export const toastQueue = new ToastQueue();

export function notify(input: ToastInput | string): string | null {
  return toastQueue.notify(input);
}

export function dismissToast(id: string): void {
  toastQueue.dismiss(id);
}

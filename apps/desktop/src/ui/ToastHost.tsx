import { Check, Info, X } from 'lucide-react';
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import { useI18n } from '../i18n';
import {
  dismissToast,
  notify,
  parseDuration,
  toastQueue,
  type ToastInput,
  type ToastItem,
  type ToastQueue,
} from './toast-queue';
import './toast-host.css';

export { ToastQueue, dismissToast, notify, parseDuration, toastQueue } from './toast-queue';
export type {
  ToastInput,
  ToastItem,
  ToastQueueOptions,
  ToastSnapshot,
  ToastTone,
} from './toast-queue';

/**
 * Shell-level transient messages.
 *
 * Mount once in the app shell; it renders nothing of its own and portals the
 * stack into the surrounding `dialog` when there is one, so a toast raised from
 * inside a modal stays above that modal instead of behind its backdrop.
 *
 * Push into the shared queue from anywhere:
 *   notify({ text: t('已保存', 'Saved'), tone: 'success' });
 * Blocking errors never come here: they stay inline with `role="alert"`.
 */
export function ToastHost({
  queue = toastQueue,
  closeLabel,
}: {
  queue?: ToastQueue;
  closeLabel?: string;
}) {
  const { t } = useI18n();
  const anchor = useRef<HTMLSpanElement>(null);
  const [target, setTarget] = useState<HTMLElement | null>(null);
  const snapshot = useSyncExternalStore(queue.subscribe, queue.getSnapshot, queue.getSnapshot);

  useEffect(() => {
    const element = anchor.current;
    if (!element) return;
    const updateTarget = () =>
      setTarget(
        element.closest('dialog') ??
          (document.fullscreenElement as HTMLElement | null) ??
          document.body,
      );
    updateTarget();
    document.addEventListener('fullscreenchange', updateTarget);
    // The CSS token is the source of truth for the countdown; the queue only
    // needs to hear about it, and running toasts keep the lifetime they started with.
    const lifetime = parseDuration(getComputedStyle(element).getPropertyValue('--toast-lifetime'));
    if (lifetime !== null) queue.setDefaultLifetime(lifetime);
    return () => document.removeEventListener('fullscreenchange', updateTarget);
  }, [queue]);

  const label = closeLabel ?? t('关闭提示', 'Dismiss notification');
  const stack = (
    <div
      className="ui-toast-host"
      role="status"
      aria-live="polite"
      aria-atomic="false"
      onPointerEnter={() => queue.pauseAll()}
      onPointerLeave={() => queue.resumeAll()}
      onFocus={() => queue.pauseAll()}
      onBlur={(event) => {
        // Moving focus between two toasts stays inside the host: keep them paused.
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) queue.resumeAll();
      }}
    >
      {snapshot.visible.map((item) => (
        <div key={item.id} className={`ui-toast is-${item.tone}`}>
          <span className="ui-toast-icon" aria-hidden="true">
            {item.tone === 'success' ? (
              <Check size={15} strokeWidth={2.2} />
            ) : (
              <Info size={15} strokeWidth={2.2} />
            )}
          </span>
          <span className="ui-toast-text">{item.text}</span>
          {item.dismissible && (
            <button
              type="button"
              className="icon-button ui-toast-close"
              aria-label={label}
              onClick={() => queue.dismiss(item.id)}
            >
              <X size={15} aria-hidden="true" />
            </button>
          )}
        </div>
      ))}
    </div>
  );

  return (
    <>
      <span ref={anchor} className="ui-toast-anchor" />
      {target ? createPortal(stack, target) : null}
    </>
  );
}

/** Queue snapshot plus the two imperative calls, for call sites that render state. */
export function useToast(): {
  visible: readonly ToastItem[];
  queued: readonly ToastItem[];
  notify: (input: ToastInput | string) => string | null;
  dismiss: (id: string) => void;
} {
  const snapshot = useSyncExternalStore(
    toastQueue.subscribe,
    toastQueue.getSnapshot,
    toastQueue.getSnapshot,
  );
  return {
    visible: snapshot.visible,
    queued: snapshot.queued,
    notify,
    dismiss: dismissToast,
  };
}

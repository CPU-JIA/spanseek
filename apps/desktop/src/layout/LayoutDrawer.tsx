import { useEffect, useRef, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { shouldBlockDrawerCancel, shouldRestoreDrawerFocus } from './drawer-policy';

/** Undocked navigation stays reachable without taking width from the document. */
export function LayoutDrawer({
  overlay,
  label,
  closeLabel,
  onDismiss,
  children,
}: {
  overlay: boolean;
  label: string;
  closeLabel: string;
  onDismiss: () => void;
  children: ReactNode;
}) {
  if (!overlay) return children;
  return (
    <Drawer label={label} closeLabel={closeLabel} onDismiss={onDismiss}>
      {children}
    </Drawer>
  );
}

function Drawer({
  label,
  closeLabel,
  onDismiss,
  children,
}: Omit<Parameters<typeof LayoutDrawer>[0], 'overlay'>) {
  const dialog = useRef<HTMLDialogElement>(null);
  const composing = useRef(false);
  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    element.showModal();
    return () => {
      if (element.open) element.close();
      if (
        previous &&
        shouldRestoreDrawerFocus({
          previous,
          activeIsBody: document.activeElement === document.body,
          activeInsideDrawer: element.contains(document.activeElement),
        })
      ) {
        previous.focus({ preventScroll: true });
      }
    };
  }, []);
  return (
    <dialog
      ref={dialog}
      className="layout-drawer"
      aria-label={label}
      onCompositionStart={() => {
        composing.current = true;
      }}
      onCompositionEnd={() => {
        // The Escape that ends the input can be delivered after `compositionend`,
        // so the flag survives until the next task — same reason NoteEditor defers
        // its composing flag.
        setTimeout(() => {
          composing.current = false;
        }, 0);
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && (event.nativeEvent.isComposing || event.keyCode === 229)) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
      onCancel={(event) => {
        // Escape is the dialog's own close gesture: it fires `cancel` without a
        // keydown we could veto, and the IME owns Escape while composing.
        if (shouldBlockDrawerCancel({ composing: composing.current })) event.preventDefault();
      }}
      onClose={(event) => {
        if (!event.currentTarget.open) onDismiss();
      }}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return;
        const bounds = event.currentTarget.getBoundingClientRect();
        if (event.clientX < bounds.left || event.clientX > bounds.right) onDismiss();
      }}
    >
      <header className="layout-drawer-header">
        <strong>{label}</strong>
        <button className="icon-button" aria-label={closeLabel} autoFocus onClick={onDismiss}>
          <X size={18} />
        </button>
      </header>
      {children}
    </dialog>
  );
}

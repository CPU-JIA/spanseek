import {
  Children,
  cloneElement,
  isValidElement,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import './tooltip.css';

/** Delayed pointer hints; keyboard users get an immediate, described hint. */
export function Tooltip({
  children,
  label,
  shortcut,
}: {
  children: ReactNode;
  label: string;
  shortcut?: string;
}) {
  const id = useId();
  const anchor = useRef<HTMLSpanElement>(null);
  const bubble = useRef<HTMLDivElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  function close() {
    clearTimeout(timer.current);
    setOpen(false);
    setPosition(null);
  }
  useEffect(() => () => clearTimeout(timer.current), []);
  useLayoutEffect(() => {
    if (!open) return;
    const target = anchor.current;
    const hint = bubble.current;
    if (!target || !hint) return;
    const rect = target.getBoundingClientRect();
    const size = hint.getBoundingClientRect();
    setPosition({
      left: Math.max(
        8,
        Math.min(rect.left + (rect.width - size.width) / 2, window.innerWidth - size.width - 8),
      ),
      top:
        rect.bottom + size.height + 8 <= window.innerHeight - 8
          ? rect.bottom + 8
          : Math.max(8, rect.top - size.height - 8),
    });
    const dismiss = () => close();
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close();
    };
    window.addEventListener('resize', dismiss);
    window.addEventListener('scroll', dismiss, true);
    document.addEventListener('keydown', escape);
    return () => {
      window.removeEventListener('resize', dismiss);
      window.removeEventListener('scroll', dismiss, true);
      document.removeEventListener('keydown', escape);
    };
  }, [open, label, shortcut]);
  const content = Children.map(children, (child) => {
    if (!isValidElement(child)) return child;
    const element = child as ReactElement<{ 'aria-describedby'?: string }>;
    return cloneElement(element, {
      'aria-describedby':
        [element.props['aria-describedby'], open ? id : null].filter(Boolean).join(' ') ||
        undefined,
    });
  });
  return (
    <span
      ref={anchor}
      className="ui-tooltip-anchor"
      onPointerEnter={(event) => {
        if (event.pointerType === 'touch') return;
        clearTimeout(timer.current);
        timer.current = setTimeout(() => setOpen(true), 450);
      }}
      onPointerLeave={close}
      onPointerDown={close}
      onFocus={(event) => {
        if (event.target.matches(':focus-visible')) {
          clearTimeout(timer.current);
          setOpen(true);
        }
      }}
      onBlur={close}
    >
      {content}
      {open &&
        createPortal(
          <div
            ref={bubble}
            id={id}
            role="tooltip"
            className="ui-tooltip"
            style={{
              left: position?.left ?? 0,
              top: position?.top ?? 0,
              visibility: position ? 'visible' : 'hidden',
            }}
          >
            <span>{label}</span>
            {shortcut && <kbd>{shortcut}</kbd>}
          </div>,
          anchor.current?.closest('dialog') ?? document.fullscreenElement ?? document.body,
        )}
    </span>
  );
}

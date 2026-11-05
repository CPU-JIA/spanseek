import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Check } from 'lucide-react';
import { Tooltip } from './Tooltip';
import './popup-menu.css';

export interface PopupMenuItem {
  id: string;
  label: string;
  /** A second, quieter line: why an entry is disabled, or what it will do. */
  detail?: string;
  icon?: ReactNode;
  shortcut?: string;
  checked?: boolean;
  radio?: boolean;
  disabled?: boolean;
  separatorBefore?: boolean;
  groupLabel?: string;
  onSelect: () => void;
}

export function PopupMenu({
  label,
  trigger,
  items,
  disabled,
  className = 'icon-button',
  placement = 'top-start',
  menuClassName,
}: {
  label: string;
  trigger: ReactNode;
  items: PopupMenuItem[];
  disabled?: boolean;
  className?: string;
  placement?: 'top-start' | 'top-end' | 'bottom-start';
  menuClassName?: string;
}) {
  const id = useId();
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const initialFocus = useRef<'first' | 'last' | 'selected'>('selected');
  const typeahead = useRef({ text: '', time: 0 });
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  function close(restoreFocus: boolean) {
    setOpen(false);
    setPosition(null);
    if (restoreFocus) button.current?.focus({ preventScroll: true });
  }
  function enabledButtons() {
    return [...(menu.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])];
  }
  useLayoutEffect(() => {
    if (!open) return;
    function positionMenu() {
      if (!button.current || !menu.current) return;
      const anchor = button.current.getBoundingClientRect();
      const bounds = menu.current.getBoundingClientRect();
      const above = anchor.top - bounds.height - 8;
      const below = anchor.bottom + 8;
      const preferAbove = placement.startsWith('top');
      const top =
        preferAbove && above >= 8
          ? above
          : !preferAbove && below + bounds.height <= window.innerHeight - 8
            ? below
            : above >= 8
              ? above
              : Math.min(below, window.innerHeight - bounds.height - 8);
      setPosition({
        left: Math.max(
          8,
          Math.min(
            placement.endsWith('end') ? anchor.right - bounds.width : anchor.left,
            window.innerWidth - bounds.width - 8,
          ),
        ),
        top: Math.max(8, top),
      });
    }
    positionMenu();
    const observer = new ResizeObserver(positionMenu);
    if (button.current) observer.observe(button.current);
    if (menu.current) observer.observe(menu.current);
    window.addEventListener('resize', positionMenu);
    window.addEventListener('scroll', positionMenu, true);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', positionMenu);
      window.removeEventListener('scroll', positionMenu, true);
    };
  }, [open, placement]);
  const positioned = position !== null;
  useLayoutEffect(() => {
    if (!open || !positioned) return;
    // The first render is hidden for measurement; focus only after it becomes visible.
    const choices = enabledButtons();
    const selected = choices.find((item) => item.getAttribute('aria-checked') === 'true');
    const target =
      initialFocus.current === 'last'
        ? choices.at(-1)
        : initialFocus.current === 'selected'
          ? (selected ?? choices[0])
          : choices[0];
    // An empty/all-disabled menu must still receive Escape and Tab instead of
    // leaving keyboard input in the editor behind the popup.
    (target ?? menu.current)?.focus({ preventScroll: true });
  }, [open, positioned]);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!menu.current?.contains(target) && !button.current?.contains(target)) close(false);
    };
    const focusOutside = (event: FocusEvent) => {
      const target = event.target as Node;
      if (!menu.current?.contains(target) && !button.current?.contains(target)) close(false);
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('focusin', focusOutside);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('focusin', focusOutside);
    };
  }, [open]);
  useEffect(() => {
    if (disabled) close(false);
  }, [disabled]);
  return (
    <>
      <Tooltip label={label}>
        <button
          ref={button}
          type="button"
          className={className}
          disabled={disabled}
          aria-label={label}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-controls={open ? id : undefined}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            initialFocus.current = 'selected';
            if (open) close(false);
            else setOpen(true);
          }}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing || event.keyCode === 229) return;
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
              event.preventDefault();
              initialFocus.current = event.key === 'ArrowDown' ? 'first' : 'last';
              setOpen(true);
            }
          }}
        >
          {trigger}
        </button>
      </Tooltip>
      {open &&
        createPortal(
          <div
            ref={menu}
            id={id}
            role="menu"
            tabIndex={-1}
            aria-label={label}
            className={['ui-popup-menu', menuClassName].filter(Boolean).join(' ')}
            style={{
              left: position?.left ?? 0,
              top: position?.top ?? 0,
              visibility: position ? 'visible' : 'hidden',
            }}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing || event.keyCode === 229) return;
              if (event.key === 'Escape') {
                event.preventDefault();
                event.stopPropagation();
                close(true);
                return;
              }
              if (event.key === 'Tab') {
                close(true);
                return;
              }
              const choices = enabledButtons();
              if (!choices.length) return;
              const current = choices.indexOf(document.activeElement as HTMLButtonElement);
              let next: number;
              if (event.key === 'ArrowDown') next = (current + 1) % choices.length;
              else if (event.key === 'ArrowUp')
                next = (current - 1 + choices.length) % choices.length;
              else if (event.key === 'Home') next = 0;
              else if (event.key === 'End') next = choices.length - 1;
              else if (
                event.key.length === 1 &&
                !event.ctrlKey &&
                !event.metaKey &&
                !event.altKey
              ) {
                const now = Date.now();
                const previous = now - typeahead.current.time < 700 ? typeahead.current.text : '';
                const text = previous + event.key.toLocaleLowerCase();
                typeahead.current = { text, time: now };
                const query = [...text].every((char) => char === text[0]) ? text[0] : text;
                for (let offset = 1; offset <= choices.length; offset++) {
                  const index =
                    ((query.length === 1 ? current : -1) + offset + choices.length) %
                    choices.length;
                  const label = choices[index].querySelector('.ui-menu-label')?.textContent ?? '';
                  if (label.toLocaleLowerCase().startsWith(query)) {
                    event.preventDefault();
                    choices[index].focus();
                    break;
                  }
                }
                return;
              } else return;
              event.preventDefault();
              choices[next]?.focus();
            }}
          >
            {items.map((item) => (
              <div key={item.id} role="none">
                {item.separatorBefore && <div className="ui-menu-divider" role="separator" />}
                {item.groupLabel && (
                  <div className="ui-menu-group-label" role="presentation">
                    {item.groupLabel}
                  </div>
                )}
                <button
                  type="button"
                  role={
                    item.checked === undefined
                      ? 'menuitem'
                      : item.radio
                        ? 'menuitemradio'
                        : 'menuitemcheckbox'
                  }
                  aria-checked={item.checked}
                  disabled={item.disabled}
                  tabIndex={-1}
                  className="ui-menu-item"
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => {
                    // Restore the trigger first. Editor commands can then return focus to
                    // the preserved ProseMirror selection without a later focus override.
                    close(true);
                    item.onSelect();
                  }}
                >
                  <span className="ui-menu-icon" aria-hidden="true">
                    {item.icon}
                  </span>
                  <span className="ui-menu-label" title={item.detail ?? item.label}>
                    {item.label}
                    {item.detail && <small className="ui-menu-detail">{item.detail}</small>}
                  </span>
                  {item.shortcut && <kbd>{item.shortcut}</kbd>}
                  {item.checked !== undefined && (
                    <span className="ui-menu-check">
                      {item.checked && <Check size={14} strokeWidth={1.8} />}
                    </span>
                  )}
                </button>
              </div>
            ))}
          </div>,
          button.current?.closest('dialog') ?? document.fullscreenElement ?? document.body,
        )}
    </>
  );
}

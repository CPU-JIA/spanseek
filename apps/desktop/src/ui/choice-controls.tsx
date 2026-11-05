import {
  Children,
  isValidElement,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import { Check } from 'lucide-react';
import './choice-controls.css';

export interface ChoiceOption {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
  searchText?: string;
  group?: string;
}

function textContent(value: ReactNode): string {
  if (typeof value === 'string' || typeof value === 'number') return String(value);
  if (Array.isArray(value)) return value.map(textContent).join('');
  if (isValidElement<{ children?: ReactNode }>(value)) return textContent(value.props.children);
  return '';
}

/** Keep native option/optgroup declarations usable while rendering one shared listbox. */
export function choiceOptions(
  children: ReactNode,
  group?: string,
  disabled = false,
): ChoiceOption[] {
  return Children.toArray(children).flatMap((child) => {
    if (
      !isValidElement<{
        children?: ReactNode;
        value?: string | number;
        label?: string;
        disabled?: boolean;
      }>(child)
    )
      return [];
    if (child.type === 'option') {
      const label = child.props.label ?? textContent(child.props.children);
      return [
        {
          value: String(child.props.value ?? label),
          label,
          group,
          disabled: disabled || child.props.disabled,
        },
      ];
    }
    return choiceOptions(
      child.props.children,
      child.type === 'optgroup' ? child.props.label : group,
      disabled || !!child.props.disabled,
    );
  });
}

export function matchingChoice(option: ChoiceOption, query: string) {
  return `${option.label} ${option.value} ${option.group ?? ''} ${option.searchText ?? ''}`
    .toLocaleLowerCase()
    .includes(query.trim().toLocaleLowerCase());
}

export function moveChoice(options: ChoiceOption[], current: number, direction: 1 | -1) {
  if (!options.length) return -1;
  for (let step = 1; step <= options.length; step++) {
    const next = (current + direction * step + options.length) % options.length;
    if (!options[next].disabled) return next;
  }
  return -1;
}

export function useChoicePopup(
  open: boolean,
  anchor: RefObject<HTMLElement | null>,
  panel: RefObject<HTMLDivElement | null>,
  dismiss: () => void,
) {
  const [position, setPosition] = useState<{
    left: number;
    top: number;
    width: number;
    maxHeight: number;
  } | null>(null);
  const close = useRef(dismiss);
  close.current = dismiss;
  useLayoutEffect(() => {
    if (!open) {
      setPosition(null);
      return;
    }
    const trigger = anchor.current;
    const popup = panel.current;
    if (!trigger || !popup) return;
    function place() {
      if (!trigger || !popup) return;
      const rect = trigger.getBoundingClientRect();
      const viewport = window.visualViewport;
      const width = viewport?.width ?? window.innerWidth;
      const height = viewport?.height ?? window.innerHeight;
      const offsetLeft = viewport?.offsetLeft ?? 0;
      const offsetTop = viewport?.offsetTop ?? 0;
      const margin = 8;
      const gap = 4;
      const below = offsetTop + height - rect.bottom - margin - gap;
      const above = rect.top - offsetTop - margin - gap;
      const desired = Math.min(popup.scrollHeight || 320, 360);
      const opensAbove = below < Math.min(desired, 160) && above > below;
      const maxHeight = Math.max(48, Math.min(360, opensAbove ? above : below));
      const popupHeight = Math.min(desired, maxHeight);
      const labels = popup.querySelectorAll<HTMLElement>('.ui-choice-option-text > span');
      const labelWidth = Array.from(labels).reduce(
        (largest, label) => Math.max(largest, label.scrollWidth),
        0,
      );
      const popupWidth = Math.min(
        Math.max(rect.width, 208, Math.min(labelWidth + 64, 520)),
        width - margin * 2,
      );
      const next = {
        left: Math.max(
          offsetLeft + margin,
          Math.min(rect.left, offsetLeft + width - popupWidth - margin),
        ),
        top: Math.max(
          offsetTop + margin,
          opensAbove ? rect.top - gap - popupHeight : rect.bottom + gap,
        ),
        width: popupWidth,
        maxHeight,
      };
      setPosition((previous) =>
        previous &&
        previous.left === next.left &&
        previous.top === next.top &&
        previous.width === next.width &&
        previous.maxHeight === next.maxHeight
          ? previous
          : next,
      );
    }
    place();
    const observer = new ResizeObserver(place);
    observer.observe(trigger);
    observer.observe(popup);
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    window.visualViewport?.addEventListener('resize', place);
    window.visualViewport?.addEventListener('scroll', place);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
      window.visualViewport?.removeEventListener('resize', place);
      window.visualViewport?.removeEventListener('scroll', place);
    };
  }, [open, anchor, panel]);
  useEffect(() => {
    if (!open) return;
    function outside(event: Event) {
      const target = event.target as Node;
      if (!anchor.current?.contains(target) && !panel.current?.contains(target)) close.current();
    }
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('focusin', outside);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('focusin', outside);
    };
  }, [open, anchor, panel]);
  return {
    container:
      anchor.current?.closest('dialog') ??
      (typeof document === 'undefined' ? null : (document.fullscreenElement ?? document.body)),
    style: {
      left: position?.left ?? 0,
      top: position?.top ?? 0,
      width: position?.width,
      maxHeight: position?.maxHeight,
      visibility: position ? ('visible' as const) : ('hidden' as const),
    },
  };
}

export function ChoiceList({
  id,
  options,
  value,
  active,
  onActive,
  onSelect,
  emptyText,
  labelledBy,
  label,
  viewportKey,
}: {
  id: string;
  options: ChoiceOption[];
  value: string;
  active: number;
  onActive: (index: number) => void;
  onSelect: (option: ChoiceOption) => void;
  emptyText: string;
  labelledBy?: string;
  label?: string;
  viewportKey: string;
}) {
  const list = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    function revealActive() {
      const item = list.current?.querySelector<HTMLElement>('[data-active="true"]');
      // Scroll only the option list, never a page/dialog behind its portal.
      if (item && list.current) {
        const itemRect = item.getBoundingClientRect();
        const listRect = list.current.getBoundingClientRect();
        if (itemRect.top < listRect.top) list.current.scrollTop -= listRect.top - itemRect.top;
        else if (itemRect.bottom > listRect.bottom)
          list.current.scrollTop += itemRect.bottom - listRect.bottom;
      }
    }
    revealActive();
    // Placement constrains the list after its first measurement. Reveal again in
    // that final viewport, including when a dialog or async results resize it.
    const frame = requestAnimationFrame(revealActive);
    const observer = new ResizeObserver(revealActive);
    if (list.current) observer.observe(list.current);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [active, options, viewportKey]);
  return (
    <div
      ref={list}
      id={id}
      role="listbox"
      aria-labelledby={labelledBy}
      aria-label={label}
      className="ui-choice-list"
    >
      {!options.length ? (
        <div className="ui-choice-empty" role="presentation">
          {emptyText}
        </div>
      ) : null}
      {options.map((option, index) => (
        <div key={option.value} role="none">
          {option.group && option.group !== options[index - 1]?.group ? (
            <div className="ui-choice-group" role="presentation">
              {option.group}
            </div>
          ) : null}
          <div
            id={`${id}-option-${index}`}
            role="option"
            aria-label={option.group ? `${option.group} · ${option.label}` : undefined}
            aria-describedby={option.description ? `${id}-description-${index}` : undefined}
            aria-selected={option.value === value}
            aria-disabled={option.disabled || undefined}
            data-active={index === active}
            className="ui-choice-option"
            title={option.description ? `${option.label}\n${option.description}` : option.label}
            onPointerMove={() => {
              if (!option.disabled && active !== index) onActive(index);
            }}
            onPointerDown={(event) => event.preventDefault()}
            onClick={() => {
              if (!option.disabled) onSelect(option);
            }}
          >
            <span className="ui-choice-option-text">
              <span>{option.label}</span>
              {option.description ? (
                <small id={`${id}-description-${index}`}>{option.description}</small>
              ) : null}
            </span>
            <span className="ui-choice-check" aria-hidden="true">
              {option.value === value ? <Check size={14} strokeWidth={1.8} /> : null}
            </span>
          </div>
        </div>
      ))}
    </div>
  );
}

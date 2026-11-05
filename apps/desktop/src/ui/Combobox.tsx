import {
  forwardRef,
  useEffect,
  useId,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type InputHTMLAttributes,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import { ChevronDown } from 'lucide-react';
import { useI18n } from '../i18n';
import {
  ChoiceList,
  matchingChoice,
  moveChoice,
  useChoicePopup,
  type ChoiceOption,
} from './choice-controls';

export type { ChoiceOption } from './choice-controls';

export interface ComboboxProps extends Omit<
  InputHTMLAttributes<HTMLInputElement>,
  'value' | 'onChange' | 'size'
> {
  value: string;
  onValueChange: (value: string) => void;
  options: ChoiceOption[];
  allowCustom?: boolean;
  filterOptions?: boolean;
  onQueryChange?: (query: string) => void;
  onOpenChange?: (open: boolean) => void;
  loading?: boolean;
  emptyText?: string;
  footer?: ReactNode;
}

/** Editable search; only allowCustom commits unlisted text to the caller's draft. */
export const Combobox = forwardRef<HTMLInputElement, ComboboxProps>(function Combobox(
  {
    value,
    onValueChange,
    options,
    allowCustom = false,
    filterOptions = true,
    onQueryChange,
    onOpenChange,
    loading,
    emptyText,
    footer,
    disabled,
    className,
    id: suppliedId,
    onKeyDown,
    onFocus,
    onBlur,
    onClick,
    ...props
  },
  ref,
) {
  const { t } = useI18n();
  const generatedId = useId();
  const id = suppliedId ?? generatedId;
  const listId = `${id}-list`;
  const input = useRef<HTMLInputElement>(null);
  const anchor = useRef<HTMLSpanElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  useImperativeHandle(ref, () => input.current!);
  const selectedLabel = allowCustom
    ? value
    : (options.find((option) => option.value === value)?.label ?? value);
  const [query, setQuery] = useState(selectedLabel);
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [active, setActive] = useState(-1);
  const visible = useMemo(
    () =>
      !filterOptions || !editing
        ? options
        : options.filter((option) => matchingChoice(option, query)),
    [options, filterOptions, editing, query],
  );
  function changeOpen(next: boolean) {
    setOpen(next);
    if (open !== next) onOpenChange?.(next);
  }
  function close() {
    changeOpen(false);
    setEditing(false);
    if (!allowCustom) setQuery(selectedLabel);
  }
  const popup = useChoicePopup(open, anchor, panel, close);
  useEffect(() => {
    if (!editing) setQuery(selectedLabel);
  }, [selectedLabel, editing]);
  useEffect(() => {
    if (disabled) close();
  }, [disabled]);
  useEffect(() => {
    if (!open) return;
    if (active >= visible.length || visible[active]?.disabled) setActive(-1);
  }, [open, active, visible]);
  function select(option: ChoiceOption) {
    setQuery(allowCustom ? option.value : option.label);
    setEditing(false);
    changeOpen(false);
    input.current?.focus({ preventScroll: true });
    onValueChange(option.value);
  }
  function footerControls() {
    return [
      ...(panel.current?.querySelectorAll<HTMLElement>(
        '.ui-choice-footer :is(button, input, select, textarea, a[href], [tabindex]):not(:disabled):not([tabindex="-1"])',
      ) ?? []),
    ].filter((element) => element.getClientRects().length > 0);
  }
  return (
    <span
      ref={anchor}
      className={['ui-combobox', className].filter(Boolean).join(' ')}
      data-disabled={disabled || undefined}
    >
      <input
        {...props}
        ref={input}
        id={id}
        type="text"
        role="combobox"
        className="ui-combobox-input"
        value={editing ? query : selectedLabel}
        disabled={disabled}
        autoComplete="off"
        aria-autocomplete="list"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={
          open && active >= 0 && visible[active] ? `${listId}-option-${active}` : undefined
        }
        onFocus={(event) => onFocus?.(event)}
        onBlur={(event) => {
          onBlur?.(event);
          if (
            !panel.current?.contains(event.relatedTarget as Node) &&
            !anchor.current?.contains(event.relatedTarget as Node)
          )
            close();
        }}
        onClick={(event) => {
          onClick?.(event);
          if (!event.defaultPrevented && !open) {
            setActive(-1);
            changeOpen(true);
          }
        }}
        onChange={(event) => {
          const text = event.target.value;
          setQuery(text);
          setEditing(true);
          setActive(-1);
          changeOpen(true);
          onQueryChange?.(text);
          if (allowCustom) onValueChange(text);
        }}
        onKeyDown={(event) => {
          onKeyDown?.(event);
          if (event.defaultPrevented || event.nativeEvent.isComposing || event.keyCode === 229)
            return;
          if (event.key === 'Escape' && open) {
            event.preventDefault();
            event.stopPropagation();
            close();
            return;
          }
          if (event.key === 'Tab') {
            const firstFooter = open && !event.shiftKey ? footerControls()[0] : undefined;
            if (firstFooter) {
              event.preventDefault();
              firstFooter.focus({ preventScroll: true });
            } else close();
            return;
          }
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            event.stopPropagation();
            if (event.altKey && event.key === 'ArrowUp' && open) {
              close();
              return;
            }
            const direction = event.key === 'ArrowDown' ? 1 : -1;
            setActive(moveChoice(visible, active < 0 && direction === -1 ? 0 : active, direction));
            changeOpen(true);
            return;
          }
          // Home/End retain native caret movement while entering a query.
          if (open && active >= 0 && (event.key === 'Home' || event.key === 'End')) {
            event.preventDefault();
            event.stopPropagation();
            setActive(
              moveChoice(visible, event.key === 'Home' ? -1 : 0, event.key === 'Home' ? 1 : -1),
            );
            return;
          }
          if (event.key === 'Enter' && open) {
            event.preventDefault();
            event.stopPropagation();
            if (active >= 0 && visible[active] && !visible[active].disabled)
              select(visible[active]);
            else if (allowCustom) close();
            else {
              const exact = visible.find(
                (option) => !option.disabled && (option.label === query || option.value === query),
              );
              if (exact) select(exact);
            }
          }
        }}
      />
      <button
        type="button"
        tabIndex={-1}
        disabled={disabled}
        className="ui-combobox-toggle"
        aria-label={t('显示选项', 'Show options')}
        aria-expanded={open}
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => {
          input.current?.focus({ preventScroll: true });
          if (open) close();
          else {
            setActive(-1);
            setEditing(false);
            changeOpen(true);
          }
        }}
      >
        <ChevronDown size={14} aria-hidden="true" />
      </button>
      {open &&
        popup.container &&
        createPortal(
          <div
            ref={panel}
            className="ui-choice-panel"
            style={popup.style}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing || event.keyCode === 229) return;
              if (event.key === 'Escape') {
                event.preventDefault();
                event.stopPropagation();
                close();
                input.current?.focus({ preventScroll: true });
              } else if (event.key === 'Tab') {
                const controls = footerControls();
                if (event.shiftKey && document.activeElement === controls[0]) {
                  event.preventDefault();
                  input.current?.focus({ preventScroll: true });
                } else if (!event.shiftKey && document.activeElement === controls.at(-1)) {
                  // Restore the anchor before default Tab navigation continues in
                  // document order, instead of following the portal at body end.
                  input.current?.focus({ preventScroll: true });
                  close();
                }
              }
            }}
          >
            <ChoiceList
              id={listId}
              options={visible}
              value={value}
              active={active}
              onActive={setActive}
              onSelect={select}
              emptyText={
                loading
                  ? t('正在读取…', 'Loading…')
                  : (emptyText ?? t('未找到选项', 'No matching options'))
              }
              labelledBy={props['aria-labelledby'] ?? (props['aria-label'] ? undefined : id)}
              label={props['aria-label']}
              viewportKey={`${popup.style.width}/${popup.style.maxHeight}`}
            />
            {allowCustom && editing && !visible.some((option) => option.value === query) ? (
              <div className="ui-choice-hint">
                {t('直接输入即可使用自定义值', 'Use your own value by typing')}
              </div>
            ) : null}
            {loading && visible.length ? (
              <div className="ui-choice-hint" role="status">
                {t('正在读取…', 'Loading…')}
              </div>
            ) : null}
            {footer ? <div className="ui-choice-footer">{footer}</div> : null}
          </div>,
          popup.container,
        )}
    </span>
  );
});

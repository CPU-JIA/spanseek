import {
  forwardRef,
  useEffect,
  useId,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import { ChevronDown } from 'lucide-react';
import { useI18n } from '../i18n';
import {
  ChoiceList,
  choiceOptions,
  moveChoice,
  useChoicePopup,
  type ChoiceOption,
} from './choice-controls';

export type { ChoiceOption } from './choice-controls';

export interface SelectProps extends Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  'value' | 'onChange' | 'children'
> {
  value: string | number;
  onValueChange: (value: string) => void;
  options?: ChoiceOption[];
  children?: ReactNode;
  placeholder?: string;
  emptyText?: string;
  required?: boolean;
  onOpenChange?: (open: boolean) => void;
}

/** A select-only combobox. Focus stays on its trigger, including through a portal. */
export const Select = forwardRef<HTMLButtonElement, SelectProps>(function Select(
  {
    value,
    onValueChange,
    options: supplied,
    children,
    placeholder,
    emptyText,
    required,
    onOpenChange,
    disabled,
    className,
    id: suppliedId,
    onKeyDown,
    onClick,
    onMouseDown,
    ...props
  },
  ref,
) {
  const { t } = useI18n();
  const generatedId = useId();
  const id = suppliedId ?? generatedId;
  const listId = `${id}-list`;
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  useImperativeHandle(ref, () => trigger.current!);
  const options = useMemo(() => supplied ?? choiceOptions(children), [supplied, children]);
  const selected = options.findIndex((option) => option.value === String(value));
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const search = useRef({ text: '', time: 0 });
  function changeOpen(next: boolean) {
    setOpen(next);
    if (open !== next) onOpenChange?.(next);
  }
  function close() {
    changeOpen(false);
  }
  const popup = useChoicePopup(open, trigger, panel, close);
  useEffect(() => {
    if (disabled) close();
  }, [disabled]);
  useEffect(() => {
    if (open && (active < 0 || active >= options.length || options[active]?.disabled)) {
      setActive(
        selected >= 0 && !options[selected].disabled ? selected : moveChoice(options, -1, 1),
      );
    }
  }, [open, active, options, selected]);
  function select(option: ChoiceOption) {
    close();
    trigger.current?.focus({ preventScroll: true });
    if (option.value !== String(value)) onValueChange(option.value);
  }
  function openWith(index: number) {
    setActive(index);
    changeOpen(true);
  }
  return (
    <>
      <button
        {...props}
        id={id}
        ref={trigger}
        type="button"
        role="combobox"
        className={['ui-select', className].filter(Boolean).join(' ')}
        disabled={disabled}
        aria-required={required || undefined}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={open && active >= 0 ? `${listId}-option-${active}` : undefined}
        title={
          props.title ??
          (options[selected]?.group
            ? `${options[selected].group} · ${options[selected].label}`
            : options[selected]?.label)
        }
        onClick={(event) => {
          onClick?.(event);
          if (event.defaultPrevented) return;
          // Rich text node views are inside ProseMirror. Letting the editor's
          // mousedown handler steal focus fires the popup's focus-outside guard
          // before a choice can be made, so keep focus on the select trigger.
          trigger.current?.focus({ preventScroll: true });
          if (open) close();
          else
            openWith(
              selected >= 0 && !options[selected].disabled ? selected : moveChoice(options, -1, 1),
            );
        }}
        onPointerDown={(event) => event.stopPropagation()}
        onMouseDown={(event) => {
          onMouseDown?.(event);
          if (!event.defaultPrevented) event.preventDefault();
          event.stopPropagation();
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
            close();
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
            const current = open ? active : selected;
            openWith(moveChoice(options, current < 0 && direction === -1 ? 0 : current, direction));
            return;
          }
          if (event.key === 'Home' || event.key === 'End') {
            event.preventDefault();
            event.stopPropagation();
            openWith(
              moveChoice(options, event.key === 'Home' ? -1 : 0, event.key === 'Home' ? 1 : -1),
            );
            return;
          }
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            event.stopPropagation();
            if (open && active >= 0 && !options[active]?.disabled) select(options[active]);
            else
              openWith(
                selected >= 0 && !options[selected].disabled
                  ? selected
                  : moveChoice(options, -1, 1),
              );
            return;
          }
          if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
            event.preventDefault();
            const now = Date.now();
            const previous = now - search.current.time < 700 ? search.current.text : '';
            const text = previous + event.key.toLocaleLowerCase();
            search.current = { text, time: now };
            const query = [...text].every((char) => char === text[0]) ? text[0] : text;
            const start = query.length === 1 ? (open ? active : selected) : -1;
            for (let step = 1; step <= options.length; step++) {
              const index = (start + step + options.length) % options.length;
              if (
                !options[index].disabled &&
                options[index].label.toLocaleLowerCase().startsWith(query)
              ) {
                openWith(index);
                break;
              }
            }
          }
        }}
      >
        <span className="ui-choice-value">
          {options[selected]?.label ?? (String(value) || placeholder || t('请选择', 'Select'))}
        </span>
        <ChevronDown size={14} aria-hidden="true" className="ui-choice-chevron" />
      </button>
      {open &&
        popup.container &&
        createPortal(
          <div ref={panel} className="ui-choice-panel" style={popup.style}>
            <ChoiceList
              id={listId}
              options={options}
              value={String(value)}
              active={active}
              onActive={setActive}
              onSelect={select}
              emptyText={emptyText ?? t('暂无选项', 'No options')}
              labelledBy={props['aria-labelledby'] ?? (props['aria-label'] ? undefined : id)}
              label={props['aria-label']}
              viewportKey={`${popup.style.width}/${popup.style.maxHeight}`}
            />
          </div>,
          popup.container,
        )}
    </>
  );
});

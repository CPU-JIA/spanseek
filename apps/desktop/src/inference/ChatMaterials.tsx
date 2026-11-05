import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import { Plus, X } from 'lucide-react';
import type { SearchEvidence, SearchScope, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { Tooltip } from '../ui/Tooltip';

export function ChatComposerPopover({
  label,
  icon,
  triggerLabel,
  children,
  disabled = false,
  initialFocus,
}: {
  label: string;
  icon: ReactNode;
  triggerLabel?: string;
  children: ReactNode;
  disabled?: boolean;
  initialFocus?: RefObject<HTMLElement | null>;
}) {
  const { t } = useI18n();
  const id = useId();
  const button = useRef<HTMLButtonElement>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const composing = useRef(false);
  const imeKey = useRef(false);
  const backdropPointer = useRef(false);
  const [open, setOpen] = useState(false);

  useLayoutEffect(() => {
    if (!open) return;
    const element = dialog.current;
    const trigger = button.current;
    if (!element || !trigger) return;
    function position() {
      if (!element || !trigger) return;
      const anchor = trigger.getBoundingClientRect();
      const bounds = element.getBoundingClientRect();
      const above = anchor.top - bounds.height - 8;
      const below = anchor.bottom + 8;
      const top =
        above >= 8
          ? above
          : below + bounds.height <= window.innerHeight - 8
            ? below
            : window.innerHeight - bounds.height - 8;
      element.style.left = `${Math.max(8, Math.min(anchor.left, window.innerWidth - bounds.width - 8))}px`;
      element.style.top = `${Math.max(8, top)}px`;
    }
    position();
    initialFocus?.current?.focus({ preventScroll: true });
    const observer = new ResizeObserver(position);
    observer.observe(element);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
    };
  }, [open, initialFocus]);

  useEffect(() => {
    if (disabled) dialog.current?.close();
  }, [disabled]);

  function outside(x: number, y: number) {
    const bounds = dialog.current?.getBoundingClientRect();
    return !!bounds && (x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom);
  }

  return (
    <>
      <Tooltip label={label}>
        <button
          ref={button}
          type="button"
          className={`chat-composer-tool${triggerLabel ? ' chat-composer-tool-labeled' : ''}`}
          disabled={disabled}
          aria-label={label}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-controls={id}
          onClick={() => {
            if (dialog.current?.open) dialog.current.close();
            else {
              composing.current = false;
              imeKey.current = false;
              dialog.current?.showModal();
              setOpen(true);
            }
          }}
        >
          {icon}
          {triggerLabel && <span>{triggerLabel}</span>}
        </button>
      </Tooltip>
      <dialog
        ref={dialog}
        id={id}
        className="chat-composer-popover"
        aria-labelledby={`${id}-title`}
        onClose={() => {
          setOpen(false);
          composing.current = false;
          imeKey.current = false;
          if (!disabled && button.current?.isConnected)
            button.current.focus({ preventScroll: true });
        }}
        onCancel={(event) => {
          event.preventDefault();
          if (!composing.current && !imeKey.current) dialog.current?.close();
        }}
        onCompositionStart={() => {
          composing.current = true;
        }}
        onCompositionEnd={() => {
          composing.current = false;
        }}
        onKeyDownCapture={(event) => {
          imeKey.current = event.nativeEvent.isComposing || event.keyCode === 229;
        }}
        onKeyDown={(event) => {
          event.stopPropagation();
          if (composing.current || imeKey.current) return;
          if (event.key === 'Enter' && event.target instanceof HTMLInputElement) {
            // Inputs in this dialog must not implicitly submit the message form.
            event.preventDefault();
          }
        }}
        onPointerDown={(event) => {
          backdropPointer.current =
            event.target === event.currentTarget && outside(event.clientX, event.clientY);
        }}
        onClick={(event) => {
          if (
            backdropPointer.current &&
            event.target === event.currentTarget &&
            outside(event.clientX, event.clientY)
          )
            dialog.current?.close();
          backdropPointer.current = false;
        }}
      >
        <div className="chat-popover-header">
          <h2 id={`${id}-title`}>{label}</h2>
          <button
            type="button"
            className="chat-popover-close"
            aria-label={t('关闭弹层', 'Close popup')}
            onClick={() => dialog.current?.close()}
          >
            <X size={16} aria-hidden="true" />
          </button>
        </div>
        <div className="chat-popover-body">{children}</div>
      </dialog>
    </>
  );
}

export function ChatMaterialChips({
  value,
  onChange,
  disabled,
}: {
  value: SearchEvidence[];
  onChange: (items: SearchEvidence[]) => void;
  disabled: boolean;
}) {
  const { t } = useI18n();
  if (!value.length) return null;
  return (
    <ul className="chat-context-chips">
      {value.map((item) => (
        <li key={item.id}>
          <span title={`${item.title}\n${item.text}`}>{item.text}</span>
          <button
            type="button"
            disabled={disabled}
            aria-label={`${t('移除资料', 'Remove material')} ${item.title}`}
            onClick={() => onChange(value.filter((entry) => entry.id !== item.id))}
          >
            ×
          </button>
        </li>
      ))}
    </ul>
  );
}

export function ChatMaterials({
  identity,
  scope,
  value,
  onChange,
  disabled,
  triggerLabel,
}: {
  identity: WorkspaceIdentity;
  scope: SearchScope;
  value: SearchEvidence[];
  onChange: (items: SearchEvidence[]) => void;
  disabled: boolean;
  triggerLabel?: string;
}) {
  const { t } = useI18n();
  const [query, setQuery] = useState('');
  const [items, setItems] = useState<SearchEvidence[]>([]);
  const [busy, setBusy] = useState(false);
  const [searched, setSearched] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sequence = useRef(0);
  const queryInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ++sequence.current;
    setItems([]);
    setSearched(false);
    setBusy(false);
    return () => {
      ++sequence.current;
    };
  }, [identity.local_instance_id, identity.workspace_id, scope]);
  async function search() {
    if (disabled || !query.trim() || busy) return;
    const current = ++sequence.current;
    setBusy(true);
    setError(null);
    try {
      const result = await call(
        {
          kind: 'search_query',
          payload: {
            query,
            mode: 'lexical',
            scope: structuredClone(scope),
            limit: 20,
            minimum_cosine: null,
          },
        },
        'search_results',
        identity,
      );
      if (current !== sequence.current) return;
      setItems(result.items);
      setSearched(true);
    } catch (e) {
      if (current === sequence.current) setError(message(e));
    } finally {
      if (current === sequence.current) setBusy(false);
    }
  }
  return (
    <ChatComposerPopover
      label={t('添加资料片段', 'Add material excerpts')}
      icon={<Plus size={18} aria-hidden="true" />}
      triggerLabel={triggerLabel}
      disabled={disabled}
      initialFocus={queryInput}
    >
      <div className="chat-materials">
        <p>
          {t(
            '搜索并选择要随本次消息发送的原文片段。展开结果可以核对实际发送的内容。',
            'Choose source excerpts to send with this message. Expand a result to inspect the exact text.',
          )}
        </p>
        <div className="chat-proposal-search">
          <input
            ref={queryInput}
            aria-label={t('搜索资料片段', 'Search material excerpts')}
            value={query}
            disabled={disabled}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (
                event.key === 'Enter' &&
                !event.nativeEvent.isComposing &&
                event.keyCode !== 229
              ) {
                event.preventDefault();
                void search();
              }
            }}
          />
          <button
            type="button"
            disabled={disabled || busy || !query.trim()}
            onClick={() => void search()}
          >
            {t('搜索', 'Search')}
          </button>
        </div>
        <p className="chat-material-count" role="status">
          {t(`已选 ${value.length} / 32 个片段`, `${value.length} / 32 excerpts selected`)}
        </p>
        {error && <p role="alert">{error}</p>}
        {searched && !items.length && (
          <p>{t('当前范围内没有匹配片段。', 'No matching excerpts in this scope.')}</p>
        )}
        <div className="chat-material-results">
          {items.map((item) => (
            <div key={item.id}>
              <details>
                <summary>{item.title}</summary>
                <p>{item.text}</p>
              </details>
              <button
                type="button"
                disabled={
                  disabled || value.length >= 32 || value.some((entry) => entry.id === item.id)
                }
                onClick={() => onChange([...value, item])}
              >
                {value.some((entry) => entry.id === item.id)
                  ? t('已加入', 'Added')
                  : t('加入', 'Add')}
              </button>
            </div>
          ))}
        </div>
      </div>
    </ChatComposerPopover>
  );
}

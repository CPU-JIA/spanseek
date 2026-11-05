import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import {
  clampCommandPaletteIndex,
  filterCommandPalette,
  moveCommandPaletteIndex,
  shouldIgnoreCommandPaletteKey,
  type CommandPaletteItem,
} from './command-palette';
import { Search } from 'lucide-react';
import { useI18n } from '../i18n';

type Props = {
  open: boolean;
  items: readonly CommandPaletteItem[];
  onClose: () => void;
};

export function CommandPalette({ open, items, onClose }: Props) {
  const { t } = useI18n();
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const composingRef = useRef(false);
  const compositionEndTimer = useRef<number | undefined>(undefined);
  const filtered = useMemo(() => filterCommandPalette(items, query), [items, query]);
  const visibleIndex = clampCommandPaletteIndex(activeIndex, filtered.length);

  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    composingRef.current = false;
    window.clearTimeout(compositionEndTimer.current);
    setQuery('');
    setActiveIndex(0);
    const focus = window.setTimeout(() => inputRef.current?.focus(), 0);
    return () => window.clearTimeout(focus);
  }, [open]);

  useEffect(
    () => () => {
      window.clearTimeout(compositionEndTimer.current);
    },
    [],
  );

  useEffect(() => {
    if (open) return;
    const target = restoreFocusRef.current;
    restoreFocusRef.current = null;
    if (target?.isConnected) target.focus();
  }, [open]);

  useEffect(() => {
    setActiveIndex((current) => {
      if (!filtered.length) return -1;
      return Math.min(Math.max(current, 0), filtered.length - 1);
    });
  }, [filtered.length]);

  if (!open) return null;

  const active = visibleIndex >= 0 ? filtered[visibleIndex] : undefined;

  function run(item: CommandPaletteItem) {
    if (item.disabled) return;
    onClose();
    // Let React remove the modal before a command checks its own modal barrier.
    window.setTimeout(() => {
      void Promise.resolve(item.run()).catch((error) => {
        console.error('Command palette command failed', error);
      });
    }, 0);
  }

  function keyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (
      shouldIgnoreCommandPaletteKey(
        { isComposing: event.nativeEvent.isComposing, keyCode: event.keyCode },
        composingRef.current,
      )
    )
      return;
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key === 'Tab') {
      event.preventDefault();
      inputRef.current?.focus();
      return;
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      setActiveIndex((current) =>
        moveCommandPaletteIndex(current, event.key === 'ArrowDown' ? 1 : -1, filtered.length),
      );
      return;
    }
    if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      setActiveIndex(event.key === 'Home' ? (filtered.length ? 0 : -1) : filtered.length - 1);
      return;
    }
    if (event.key === 'Enter' && active) {
      event.preventDefault();
      run(active);
    }
  }

  return (
    <div
      className="command-palette-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="command-palette"
        role="dialog"
        aria-modal="true"
        aria-label={t('命令面板', 'Command palette')}
        onKeyDown={keyDown}
        onCompositionStart={() => {
          composingRef.current = true;
          window.clearTimeout(compositionEndTimer.current);
        }}
        onCompositionEnd={() => {
          // The key that closes an IME composition can arrive after compositionend.
          window.clearTimeout(compositionEndTimer.current);
          compositionEndTimer.current = window.setTimeout(() => {
            composingRef.current = false;
          }, 0);
        }}
      >
        <div className="command-palette-input-row">
          <Search size={18} aria-hidden="true" />
          <input
            ref={inputRef}
            className="command-palette-input"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t('搜索命令…', 'Search commands…')}
            role="combobox"
            aria-autocomplete="list"
            aria-expanded="true"
            aria-controls="command-palette-list"
            aria-activedescendant={active ? `command-palette-${active.id}` : undefined}
            spellCheck={false}
          />
          <kbd>Esc</kbd>
        </div>
        <div id="command-palette-list" className="command-palette-list" role="listbox">
          {filtered.length ? (
            filtered.map((item, index) => {
              const selected = index === visibleIndex;
              return (
                <button
                  key={item.id}
                  id={`command-palette-${item.id}`}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  className="command-palette-option"
                  disabled={item.disabled}
                  tabIndex={-1}
                  onMouseMove={() => setActiveIndex(index)}
                  onClick={() => run(item)}
                >
                  <span className="command-palette-icon" aria-hidden="true">
                    {item.icon}
                  </span>
                  <span className="command-palette-copy">
                    <strong>{item.label}</strong>
                    {item.description && <small>{item.description}</small>}
                  </span>
                  {(item.shortcut || item.disabled) && (
                    <span className="command-palette-meta">
                      {item.shortcut && <kbd>{item.shortcut}</kbd>}
                      {item.disabled && (
                        <span className="command-palette-hint">{t('不可用', 'Unavailable')}</span>
                      )}
                    </span>
                  )}
                </button>
              );
            })
          ) : (
            <p className="command-palette-empty">
              {t('没有匹配的命令。', 'No matching commands.')}
            </p>
          )}
        </div>
        <div className="command-palette-footer">
          <span>{t('↑↓ 选择', '↑↓ Select')}</span>
          <span>{t('Enter 执行', 'Enter Run')}</span>
          <span>{t('Esc 关闭', 'Esc Close')}</span>
        </div>
      </div>
    </div>
  );
}

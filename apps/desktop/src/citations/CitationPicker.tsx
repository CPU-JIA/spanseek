import { Select } from '../ui/Select';
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { BookMarked, Check, Search, Trash2, X } from 'lucide-react';
import type { BibliographyEntry, WorkspaceIdentity } from '@pkms/contracts';
import { call, choose, message } from '../api';
import { useI18n } from '../i18n';
import { citationLabel, filterCitations } from './citation-filter';
import {
  CITATION_LOCATOR_OPTIONS,
  citationMarker,
  type CitationLocatorKind,
  type CitationSelection,
} from './citation-selection';
import '../editor/reference-ui.css';
import './citation-picker.css';

/**
 * Citation picker: searches citation-keyed reference notes, lets the writer add
 * one or more references with CSL locators, then inserts one plain-text cluster
 * such as `[@vaswani2017, pp. 3-4; @devlin2019, ch. 2]` at the cursor.
 */
export function CitationPicker({
  identity,
  onSelect,
  onClose,
}: {
  identity: WorkspaceIdentity;
  onSelect: (marker: string) => void;
  onClose: () => void;
}) {
  const { t, language } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const [entries, setEntries] = useState<BibliographyEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const [reloadKey, setReloadKey] = useState(0);
  const [importing, setImporting] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [selected, setSelected] = useState<CitationSelection[]>([]);
  const visible = useMemo(() => filterCitations(entries ?? [], query), [entries, query]);
  const marker = useMemo(() => citationMarker(selected), [selected]);
  useEffect(() => {
    dialog.current?.showModal();
    input.current?.focus();
  }, []);
  useEffect(() => {
    let live = true;
    setEntries(null);
    setError(null);
    call({ kind: 'bibliography_catalog' }, 'bibliography_catalog', identity).then(
      (value) => {
        if (live) setEntries(value);
      },
      (reason: unknown) => {
        if (live) setError(message(reason));
      },
    );
    return () => {
      live = false;
    };
  }, [identity, reloadKey]);
  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    document.getElementById(`citation-option-${active}`)?.scrollIntoView({ block: 'nearest' });
  }, [active]);
  async function importFile() {
    if (importing) return;
    setImporting(true);
    setError(null);
    setStatus(null);
    try {
      const grant = await choose('import_bibliography', identity);
      if (!grant) return;
      const report = await call(
        { kind: 'bibliography_import', payload: { token: grant.token } },
        'bib_import',
        identity,
      );
      setStatus(
        language === 'zh-CN'
          ? `已导入 ${report.imported} 条，跳过 ${report.skipped_existing} 条已有条目。`
          : `Imported ${report.imported}, skipped ${report.skipped_existing} existing.`,
      );
      setReloadKey((value) => value + 1);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setImporting(false);
    }
  }
  function addEntry(entry: BibliographyEntry | undefined) {
    if (!entry) return;
    setSelected((current) =>
      current.some((item) => item.key === entry.key)
        ? current
        : [...current, { key: entry.key, locatorKind: 'none', locatorValue: '' }],
    );
  }
  function updateSelection(
    key: string,
    patch: Partial<Pick<CitationSelection, 'locatorKind' | 'locatorValue'>>,
  ) {
    setSelected((current) =>
      current.map((item) => (item.key === key ? { ...item, ...patch } : item)),
    );
  }
  function removeSelection(key: string) {
    setSelected((current) => current.filter((item) => item.key !== key));
  }
  function insertSelected() {
    if (!marker) return;
    onSelect(marker);
    onClose();
  }
  function keyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (!visible.length) return;
      const step = event.key === 'ArrowDown' ? 1 : -1;
      setActive((current) => (current + step + visible.length) % visible.length);
      return;
    }
    if (event.key === 'Home' || event.key === 'End') {
      if (!visible.length) return;
      event.preventDefault();
      setActive(event.key === 'Home' ? 0 : visible.length - 1);
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      if (event.ctrlKey || event.metaKey) insertSelected();
      else addEntry(visible[active]);
    }
  }
  return (
    <dialog
      ref={dialog}
      className="reference-picker citation-picker"
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClose={onClose}
    >
      <header>
        <h2 id={titleId}>{t('插入文献引用', 'Insert citation')}</h2>
        <div className="citation-actions">
          <button
            type="button"
            className="text-button"
            disabled={importing}
            aria-busy={importing}
            onClick={() => void importFile()}
          >
            {importing
              ? t('正在导入…', 'Importing…')
              : t('导入 .bib / .json', 'Import .bib / .json')}
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label={t('关闭', 'Close')}
            onClick={onClose}
          >
            <X size={17} />
          </button>
        </div>
      </header>
      <label className="reference-search">
        <Search size={16} />
        <input
          ref={input}
          type="search"
          value={query}
          placeholder={t('搜索标题、作者或引用键', 'Search title, author, or key')}
          aria-label={t('搜索参考文献', 'Search references')}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={keyDown}
        />
      </label>
      {status && <p role="status">{status}</p>}
      {entries === null && !error && (
        <p role="status">{t('正在读取参考文献…', 'Loading references…')}</p>
      )}
      {error && (
        <div className="inline-error" role="alert">
          <p>{error}</p>
          <button type="button" onClick={() => setReloadKey((value) => value + 1)}>
            {t('重试', 'Retry')}
          </button>
        </div>
      )}
      {selected.length > 0 && (
        <section className="citation-selection" aria-label={t('已选引文', 'Selected citations')}>
          <h3>{t(`已选引文（${selected.length}）`, `Selected citations (${selected.length})`)}</h3>
          <div className="citation-selection-list">
            {selected.map((selection) => {
              const entry = entries?.find((item) => item.key === selection.key);
              const selectId = `citation-locator-${selection.key}`;
              const valueId = `citation-locator-value-${selection.key}`;
              return (
                <div className="citation-selection-row" key={selection.key}>
                  <div className="citation-selection-entry">
                    <BookMarked size={17} aria-hidden="true" />
                    <span>
                      <strong>{entry?.title ?? selection.key}</strong>
                      <small>@{selection.key}</small>
                    </span>
                  </div>
                  <div className="citation-locator-controls">
                    <div className="choice-field">
                      <span>{t('定位符', 'Locator')}</span>
                      <Select
                        aria-label={t('定位符', 'Locator')}
                        id={selectId}
                        value={selection.locatorKind}
                        onValueChange={(value) =>
                          updateSelection(selection.key, {
                            locatorKind: value as CitationLocatorKind,
                          })
                        }
                      >
                        {CITATION_LOCATOR_OPTIONS.map((option) => (
                          <option key={option.value} value={option.value}>
                            {language === 'zh-CN' ? option.zh : option.en}
                          </option>
                        ))}
                      </Select>
                    </div>
                    {selection.locatorKind !== 'none' && (
                      <label htmlFor={valueId}>
                        <span>{t('值', 'Value')}</span>
                        <input
                          id={valueId}
                          type="text"
                          value={selection.locatorValue}
                          placeholder={t('如 3-4、图 3', 'e.g. 3-4 or Fig. 3')}
                          onChange={(event) =>
                            updateSelection(selection.key, { locatorValue: event.target.value })
                          }
                          onKeyDown={(event) => {
                            if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                              event.preventDefault();
                              insertSelected();
                            }
                          }}
                        />
                      </label>
                    )}
                  </div>
                  <button
                    type="button"
                    className="icon-button citation-selection-remove"
                    aria-label={t(
                      `移除引用 @${selection.key}`,
                      `Remove citation @${selection.key}`,
                    )}
                    onClick={() => removeSelection(selection.key)}
                  >
                    <Trash2 size={16} aria-hidden="true" />
                  </button>
                </div>
              );
            })}
          </div>
        </section>
      )}
      {entries !== null && !entries.length && (
        <p className="muted">
          {t(
            '知识库还没有参考文献条目。导入 BibTeX 或 CSL JSON 后会显示在这里。',
            'This workspace has no reference entries yet. Imported BibTeX or CSL JSON entries appear here.',
          )}
        </p>
      )}
      {entries !== null && entries.length > 0 && (
        <div
          className="reference-target-list"
          id="citation-options"
          aria-label={t('参考文献', 'References')}
        >
          {visible.map((entry, index) => {
            const chosen = selected.some((item) => item.key === entry.key);
            return (
              <button
                type="button"
                key={entry.key}
                id={`citation-option-${index}`}
                className={`reference-target citation-option${index === active ? ' is-selected' : ''}${chosen ? ' is-chosen' : ''}`}
                aria-pressed={chosen}
                onMouseDown={(event) => event.preventDefault()}
                onMouseEnter={() => setActive(index)}
                onClick={() => addEntry(entry)}
              >
                <BookMarked size={17} aria-hidden="true" />
                <span>
                  <strong>{entry.title}</strong>
                  <small>{citationLabel(entry)}</small>
                  <small className="reference-identity">@{entry.key}</small>
                </span>
                {chosen && <Check className="citation-option-state" size={16} aria-hidden="true" />}
              </button>
            );
          })}
          {!visible.length && (
            <p className="muted">{t('没有匹配的参考文献。', 'No matching references.')}</p>
          )}
        </div>
      )}
      <div className="citation-insert-bar">
        <code aria-live="polite">
          {marker || t('选择条目后可插入', 'Choose a reference to insert')}
        </code>
        <button
          type="button"
          className="primary-button"
          disabled={!marker}
          onClick={insertSelected}
        >
          {t(`插入引用（${selected.length}）`, `Insert citation (${selected.length})`)}
        </button>
      </div>
      <p className="reference-picker-hint">
        {t(
          '↑↓ 选择，Enter 添加，Ctrl+Enter 插入，Esc 关闭',
          'Arrow keys to choose, Enter to add, Ctrl+Enter to insert, Esc to close',
        )}
      </p>
    </dialog>
  );
}

import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { SlidersHorizontal, X } from 'lucide-react';
import type { SourcesFilter, Tag, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { TagFilter } from '../organization/TagFilter';
import { Select } from '../ui/Select';
import { useChoicePopup } from '../ui/choice-controls';
import { readSelectedSourceTags } from './source-filter-tags';
import { SourceCollectionPicker, type SourceCollectionChoice } from './SourceCollectionPicker';

export const emptySourceFilter: SourcesFilter = {
  format: null,
  reading_status: null,
  inbox_state: null,
  collection_id: null,
  tag_ids: [],
};
export function SourceFilters({
  identity,
  value,
  onChange,
  generation,
}: {
  identity: WorkspaceIdentity;
  value: SourcesFilter;
  onChange: (value: SourcesFilter) => void;
  generation: number;
}) {
  const { t } = useI18n();
  const panelId = useId();
  const [tagCache, setTagCache] = useState<{ scope: string; library: string; items: Tag[] } | null>(
    null,
  );
  const [tagError, setTagError] = useState<{ scope: string; message: string } | null>(null);
  const [tagRetry, setTagRetry] = useState(0);
  const [open, setOpen] = useState(false);
  const tagTrigger = useRef<HTMLButtonElement>(null);
  const tagPanel = useRef<HTMLDivElement>(null);
  const panelOwnedFocus = useRef(false);
  const lastTagFocus = useRef<HTMLElement | null>(null);
  const popup = useChoicePopup(open, tagTrigger, tagPanel, () => setOpen(false));
  const [collection, setCollection] = useState<SourceCollectionChoice | null>(null);
  const [collectionError, setCollectionError] = useState<string | null>(null);
  const libraryScope = JSON.stringify([identity.local_instance_id, identity.workspace_id]);
  const tagScope = libraryScope + '/' + generation;
  const tagSelection = JSON.stringify(value.tag_ids);
  const selectionScope = tagScope + '/' + tagSelection;
  const tags =
    tagCache?.library === libraryScope
      ? value.tag_ids.flatMap((id) => tagCache.items.filter((tag) => tag.id === id))
      : [];
  const hasCachedSelection = tags.length === value.tag_ids.length;
  const tagsReady = hasCachedSelection && (!value.tag_ids.length || tagCache?.scope === tagScope);
  useEffect(() => {
    if (!open) return;
    const frame = requestAnimationFrame(() => {
      const panel = tagPanel.current;
      (
        panel?.querySelector<HTMLElement>('button:not(:disabled), input:not(:disabled)') ?? panel
      )?.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [open]);
  useLayoutEffect(() => {
    const panel = tagPanel.current;
    if (!open || !panel) {
      panelOwnedFocus.current = false;
      return;
    }
    const active = document.activeElement;
    if (
      panelOwnedFocus.current &&
      (active === document.body ||
        (active instanceof HTMLElement && panel.contains(active) && active.matches(':disabled')))
    )
      panel.focus({ preventScroll: true });
    if (document.activeElement !== panel) return;
    const remembered = lastTagFocus.current;
    const target =
      remembered?.isConnected && panel.contains(remembered) && !remembered.matches(':disabled')
        ? remembered
        : panel.querySelector<HTMLElement>('button:not(:disabled), input:not(:disabled)');
    target?.focus({ preventScroll: true });
  }, [open, tagsReady, tagScope, tagError?.scope, tagRetry]);
  useEffect(() => {
    if (!open || tagsReady) return;
    let live = true;
    setTagError(null);
    void readSelectedSourceTags(
      value.tag_ids,
      (cursor) =>
        call({ kind: 'tags_list', payload: { limit: 100, cursor, query: null } }, 'tags', identity),
      () => live,
    )
      .then((items) => {
        if (live && items) setTagCache({ scope: tagScope, library: libraryScope, items });
      })
      .catch((caught) => {
        if (live) setTagError({ scope: selectionScope, message: message(caught) });
      });
    return () => {
      live = false;
    };
  }, [open, tagScope, tagSelection, tagsReady, tagRetry]);
  useEffect(() => {
    let live = true;
    setCollectionError(null);
    if (!value.collection_id) {
      setCollection(null);
      return;
    }
    void call(
      { kind: 'collections_get', payload: { collection_id: value.collection_id } },
      'collection',
      identity,
    )
      .then((result) => {
        if (live) setCollection({ id: result.id, name: result.name });
      })
      .catch((caught) => {
        if (live) setCollectionError(message(caught));
      });
    return () => {
      live = false;
    };
  }, [value.collection_id, tagScope]);
  const hasConditions = Boolean(
    value.format ||
    value.reading_status ||
    value.inbox_state ||
    value.collection_id ||
    value.tag_ids.length,
  );
  return (
    <div
      className="source-filter-controls"
      aria-label={t('筛选资料', 'Filter documents')}
      onKeyDown={(event) => {
        if (
          event.key === 'Escape' &&
          open &&
          !event.nativeEvent.isComposing &&
          event.keyCode !== 229
        ) {
          event.preventDefault();
          event.stopPropagation();
          setOpen(false);
          tagTrigger.current?.focus();
        }
      }}
    >
      <Select
        aria-label={t('资料格式', 'Document format')}
        value={value.format ?? ''}
        onValueChange={(format) =>
          onChange({ ...value, format: (format || null) as SourcesFilter['format'] })
        }
      >
        <option value="">{t('全部格式', 'All formats')}</option>
        <option value="pdf">PDF</option>
        <option value="markdown">Markdown</option>
        <option value="text">TXT</option>
        <option value="docx">DOCX</option>
        <option value="doc">DOC</option>
        <option value="epub">EPUB</option>
        <option value="mobi">MOBI</option>
        <option value="csv">CSV</option>
        <option value="xlsx">XLSX</option>
        <option value="xls">XLS</option>
        <option value="pptx">PPTX</option>
        <option value="html">{t('网页', 'Webpage')}</option>
      </Select>
      <Select
        aria-label={t('阅读状态', 'Reading status')}
        value={value.reading_status ?? ''}
        onValueChange={(reading) =>
          onChange({
            ...value,
            reading_status: (reading || null) as SourcesFilter['reading_status'],
          })
        }
      >
        <option value="">{t('全部阅读状态', 'Any reading status')}</option>
        <option value="unread">{t('未读', 'Unread')}</option>
        <option value="reading">{t('阅读中', 'Reading')}</option>
        <option value="finished">{t('已读', 'Finished')}</option>
      </Select>
      <Select
        aria-label={t('整理状态', 'Organization status')}
        value={value.inbox_state ?? ''}
        onValueChange={(inbox) =>
          onChange({ ...value, inbox_state: (inbox || null) as SourcesFilter['inbox_state'] })
        }
      >
        <option value="">{t('全部整理状态', 'Any organization status')}</option>
        <option value="pending">{t('收件箱', 'Inbox')}</option>
        <option value="snoozed">{t('稍后', 'Later')}</option>
        <option value="processed">{t('已整理', 'Organized')}</option>
      </Select>
      <SourceCollectionPicker
        identity={identity}
        disabled={false}
        generation={generation}
        label={t('筛选集合', 'Filter by collection')}
        emptyLabel={t('全部集合', 'All collections')}
        value={
          value.collection_id
            ? collection?.id === value.collection_id
              ? collection
              : {
                  id: value.collection_id,
                  name: t('已选集合', 'Selected collection'),
                }
            : null
        }
        onChange={(next) => {
          setCollection(next);
          onChange({ ...value, collection_id: next?.id ?? null });
        }}
      />
      <button
        ref={tagTrigger}
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((current) => !current)}
      >
        <SlidersHorizontal size={15} aria-hidden="true" />
        {t('标签', 'Tags')}
        {value.tag_ids.length ? ' · ' + value.tag_ids.length : ''}
      </button>
      {hasConditions && (
        <button
          type="button"
          className="source-clear-filters"
          onClick={() => onChange(emptySourceFilter)}
        >
          <X size={14} aria-hidden="true" />
          {t('清除', 'Clear')}
        </button>
      )}
      {collectionError && (
        <p className="source-filter-error" role="alert">
          {collectionError}
        </p>
      )}
      {open &&
        popup.container &&
        createPortal(
          <div
            ref={tagPanel}
            className="source-filter-panel"
            tabIndex={-1}
            id={panelId}
            role="dialog"
            aria-label={t('按标签筛选', 'Filter by tag')}
            style={popup.style}
            onFocusCapture={(event) => {
              panelOwnedFocus.current = true;
              if (event.target !== event.currentTarget && event.target instanceof HTMLElement)
                lastTagFocus.current = event.target;
            }}
            onBlurCapture={(event) => {
              if (event.relatedTarget && !event.currentTarget.contains(event.relatedTarget as Node))
                panelOwnedFocus.current = false;
            }}
            onKeyDown={(event) => {
              if (event.key !== 'Tab' || event.nativeEvent.isComposing || event.keyCode === 229)
                return;
              const controls = Array.from(
                event.currentTarget.querySelectorAll<HTMLElement>(
                  'button:not(:disabled), input:not(:disabled)',
                ),
              ).filter((control) => control.tabIndex >= 0 && control.getClientRects().length > 0);
              const active = document.activeElement;
              if (event.shiftKey && (active === controls[0] || active === event.currentTarget)) {
                event.preventDefault();
                setOpen(false);
                tagTrigger.current?.focus({ preventScroll: true });
              } else if (!event.shiftKey && (active === controls.at(-1) || !controls.length)) {
                // Resume normal tab order from the anchor, not from the body portal.
                tagTrigger.current?.focus({ preventScroll: true });
                setOpen(false);
              }
            }}
          >
            {hasCachedSelection && (
              <fieldset
                className="source-tag-choices"
                disabled={!tagsReady}
                aria-busy={!tagsReady}
                style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }}
              >
                <TagFilter
                  key={libraryScope}
                  identity={identity}
                  selected={tags}
                  generation={generation}
                  onSelect={(next) => {
                    if (!tagsReady) return;
                    setTagCache({ scope: tagScope, library: libraryScope, items: next });
                    onChange({ ...value, tag_ids: next.map((tag) => tag.id).sort() });
                  }}
                />
              </fieldset>
            )}
            {!tagsReady &&
              (tagError?.scope === selectionScope ? (
                <div role="alert">
                  <p>{tagError.message}</p>
                  <button
                    type="button"
                    onClick={() => {
                      tagPanel.current?.focus({ preventScroll: true });
                      setTagRetry((current) => current + 1);
                    }}
                  >
                    {t('重新读取标签', 'Reload tags')}
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      tagPanel.current?.focus({ preventScroll: true });
                      onChange({ ...value, tag_ids: [] });
                    }}
                  >
                    {t('清除标签筛选', 'Clear tag filter')}
                  </button>
                </div>
              ) : (
                <p role="status">{t('正在读取已选标签…', 'Loading selected tags…')}</p>
              ))}
          </div>,
          popup.container,
        )}
    </div>
  );
}

import { Select } from './ui/Select';
import {
  ArrowDownWideNarrow,
  ArrowDownAZ,
  ArrowDownZA,
  ChevronDown,
  Clock3,
  FileText,
  Inbox,
  ListFilter,
  Plus,
  RefreshCw,
  Search,
  X,
  Ellipsis,
  Trash2,
  CheckSquare,
} from 'lucide-react';
import { focusActiveNoteTitle } from './editor/focus-note-title';
import type { NoteSort, NoteSummary, Tag, WorkspaceIdentity } from '@pkms/contracts';
import { displayDate } from './format';
import { ListPagination, type PagedListController } from './layout/Pagination.tsx';
import { PageState } from './layout/PageState';
import { resolveListDisplayState } from './layout/page-state';
import { useI18n } from './i18n';
import { PopupMenu } from './ui/PopupMenu';
import { useEffect, useState, type ReactNode } from 'react';
import { BulkOrganization, type BulkOrganizationProps } from './organization/BulkOrganization';
import { NoteCardPreview } from './gallery/NoteCardPreview';
import type { ObjectTypeDefinition } from './organization/object-types';
import { tagToneClass } from './ui/tag-color';

export function LibraryBrowser({
  identity,
  toolbar,
  recent,
  notes,
  width,
  title,
  gallery,
  filter,
  onFilter,
  objectTypes,
  typeFilter,
  onTypeFilter,
  tags,
  onClearTag,
  sort,
  onSort,
  activeId,
  onReload,
  onCreate,
  onOpen,
  onTrash,
  busy,
  bulk,
}: {
  identity: WorkspaceIdentity;
  toolbar?: ReactNode;
  recent?: ReactNode;
  notes: PagedListController<NoteSummary>;
  width: number;
  title: string;
  gallery: boolean;
  filter: string;
  onFilter: (value: string) => void;
  objectTypes: ObjectTypeDefinition[];
  typeFilter: string;
  onTypeFilter: (value: string) => void;
  tags: Tag[];
  onClearTag: (id?: string) => void;
  sort: NoteSort;
  onSort: (sort: NoteSort) => void;
  activeId: string | null;
  onReload: () => void;
  onCreate: () => void;
  onOpen: (id: string) => void;
  onTrash: (id: string) => void;
  busy: boolean;
  bulk?: BulkOrganizationProps;
}) {
  const { t, language } = useI18n();
  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState<{ id: string; title: string }[]>([]);
  const [listRoot, setListRoot] = useState<HTMLDivElement | null>(null);
  const [previewRetries, setPreviewRetries] = useState<Record<string, number>>({});
  const [previewErrors, setPreviewErrors] = useState<Record<string, boolean>>({});
  useEffect(() => {
    setSelecting(false);
    setSelected([]);
  }, [typeFilter]);
  const sortOptions = [
    { id: 'updated_desc', label: t('最近编辑', 'Last edited'), icon: <Clock3 /> },
    { id: 'updated_asc', label: t('最早编辑', 'Oldest edited'), icon: <Clock3 /> },
    { id: 'title_asc', label: t('标题 A → Z', 'Title A → Z'), icon: <ArrowDownAZ /> },
    { id: 'title_desc', label: t('标题 Z → A', 'Title Z → A'), icon: <ArrowDownZA /> },
  ] satisfies { id: NoteSort; label: string; icon: React.ReactNode }[];
  const items = notes.page?.items ?? [];
  const loading = notes.loading;
  const error = notes.error;
  const displayState = resolveListDisplayState({
    hasPage: Boolean(notes.page),
    itemCount: items.length,
    loading,
    error,
  });
  const status = (note: NoteSummary) =>
    note.inbox_state === 'pending'
      ? t('待整理', 'Inbox')
      : note.inbox_state === 'snoozed'
        ? t('稍后整理', 'Later')
        : t('已整理', 'Organized');
  const searchField = (
    <label className="filter-field">
      <Search size={15} />
      <input
        id="note-filter"
        aria-label={t('搜索笔记标题', 'Search note titles')}
        placeholder={t('搜索笔记标题…', 'Search note titles…')}
        maxLength={512}
        value={filter}
        onChange={(event) => onFilter(event.target.value)}
        onKeyDown={(event) => {
          if (event.key !== 'Escape') return;
          event.preventDefault();
          if (filter) {
            onFilter('');
            return;
          }
          const target =
            document.querySelector<HTMLElement>('#library-note-list .note-row') ??
            document.querySelector<HTMLElement>('.capture-button');
          target?.focus({ preventScroll: true });
        }}
      />
      {filter && (
        <button
          className="clear-filter"
          aria-label={t('清除文本筛选', 'Clear text filter')}
          onClick={() => onFilter('')}
        >
          <X size={13} />
        </button>
      )}
    </label>
  );
  return (
    <section
      id="library-note-list"
      className={`note-list ${gallery ? 'library-gallery' : 'resizable'}`}
      style={gallery ? undefined : { width, flexBasis: width }}
      aria-label={title}
    >
      <div className="list-heading">
        <div className={gallery ? 'sr-only' : undefined}>
          <h1>{title}</h1>
        </div>
        {gallery && searchField}
        {gallery && (
          <button
            className="gallery-create"
            aria-label={t('新建笔记', 'New note')}
            disabled={busy}
            onClick={onCreate}
          >
            <Plus size={16} />
            {t('笔记', 'Note')}
          </button>
        )}
        <button
          className="icon-button"
          aria-label={t('刷新笔记', 'Refresh notes')}
          title={t('刷新笔记', 'Refresh notes')}
          disabled={loading}
          onClick={onReload}
        >
          <RefreshCw size={16} className={loading ? 'spin' : ''} />
        </button>
        {gallery && <div className="gallery-view-controls">{toolbar}</div>}
        {!gallery && (
          <button
            className="icon-button"
            aria-label={t('新建笔记', 'New note')}
            title={t('新建笔记', 'New note')}
            disabled={busy}
            onClick={onCreate}
          >
            <Plus size={18} />
          </button>
        )}
      </div>
      <div className="library-controls">
        {!gallery && searchField}
        <div
          className="filter-field object-type-filter"
          role="group"
          aria-label={t('对象类型', 'Object type')}
        >
          <ListFilter size={14} aria-hidden="true" />
          <Select
            aria-label={t('按对象类型筛选', 'Filter by object type')}
            value={typeFilter}
            onValueChange={(value) => onTypeFilter(value)}
          >
            <option value="">{t('全部类型', 'All types')}</option>
            {objectTypes.map((type) => (
              <option value={type.id} key={type.id}>
                {t(type.pluralZh, type.pluralEn)}
              </option>
            ))}
          </Select>
        </div>
        <div className="sort-control">
          <PopupMenu
            label={t('笔记排序', 'Sort notes')}
            className="sort-menu-trigger"
            placement="bottom-start"
            trigger={
              <>
                <ArrowDownWideNarrow size={14} />
                <span>{sortOptions.find((option) => option.id === sort)?.label}</span>
                <ChevronDown size={13} />
              </>
            }
            items={sortOptions.map((option) => ({
              ...option,
              checked: sort === option.id,
              radio: true,
              onSelect: () => onSort(option.id),
            }))}
          />
        </div>
        {bulk && (
          <button
            className="selection-control"
            aria-pressed={selecting}
            title={t(
              '勾选会跨页保留；这里的操作只作用于当前页显示的内容。',
              'Ticks carry across pages; actions here apply to what the current page shows.',
            )}
            onClick={() => {
              setSelecting(!selecting);
              setSelected([]);
            }}
          >
            <CheckSquare size={14} />
            {selecting ? t('结束选择', 'Finish selecting') : t('选择笔记', 'Select notes')}
          </button>
        )}
      </div>
      {recent}
      {tags.length > 0 && (
        <div className="active-tag-filters">
          {tags.map((tag) => (
            <div className={`active-tag-filter ${tagToneClass(tag.id)}`} key={tag.id}>
              <span>
                <ListFilter size={13} /> {tag.display_name}
              </span>
              <button
                className="icon-button"
                aria-label={t(
                  `清除标签筛选：${tag.display_name}`,
                  `Clear tag filter: ${tag.display_name}`,
                )}
                onClick={() => onClearTag(tag.id)}
              >
                <X size={13} />
              </button>
            </div>
          ))}
          {tags.length > 1 && (
            <button className="text-button" onClick={() => onClearTag()}>
              {t('清除全部', 'Clear all')}
            </button>
          )}
        </div>
      )}
      {bulk && <BulkOrganization {...bulk} selected={selected} onClear={() => setSelected([])} />}
      <div
        ref={setListRoot}
        className={`list-items ${displayState === 'loading' || displayState === 'error' ? 'list-items-state' : ''}`}
        aria-busy={loading}
      >
        {displayState === 'error' && (
          <PageState
            kind="error"
            title={t('无法读取笔记', 'Could not load notes')}
            detail={error}
            retryLabel={t('重新读取笔记', 'Reload notes')}
            onRetry={onReload}
            compact
          />
        )}
        {displayState === 'content-with-error' && (
          <div role="alert" className="list-error">
            <p className="inline-error">{error}</p>
            <button className="text-button" onClick={onReload}>
              {t('重新读取笔记', 'Reload notes')}
            </button>
          </div>
        )}
        {displayState === 'loading' && (
          <PageState kind="loading" title={t('正在读取笔记…', 'Loading notes…')} compact />
        )}
        {items.map((note) => (
          <div className={`note-entry ${selecting ? 'selectable' : ''}`} key={note.id}>
            {selecting && (
              <label className="note-select">
                <input
                  type="checkbox"
                  aria-label={t(`选择笔记：${note.title}`, `Select note: ${note.title}`)}
                  checked={selected.some((item) => item.id === note.id)}
                  disabled={
                    busy ||
                    (selected.length >= 100 && !selected.some((item) => item.id === note.id))
                  }
                  onChange={(event) =>
                    setSelected((old) =>
                      event.target.checked
                        ? [...old, { id: note.id, title: note.title }]
                        : old.filter((item) => item.id !== note.id),
                    )
                  }
                />
              </label>
            )}
            <button
              className={`note-row ${activeId === note.id ? 'selected' : ''}`}
              aria-label={
                gallery
                  ? `${note.title || t('未命名笔记', 'Untitled note')}${previewErrors[note.id] ? t('，预览读取失败，可从笔记操作中重试', ', preview unavailable; retry from note actions') : ''}`
                  : undefined
              }
              onClick={(event) => {
                onOpen(note.id);
                if (event.detail === 0) focusActiveNoteTitle();
              }}
            >
              {gallery ? (
                <NoteCardPreview
                  identity={identity}
                  note={note}
                  root={listRoot}
                  retryToken={previewRetries[note.id] ?? 0}
                  onPreviewState={(state) =>
                    setPreviewErrors((old) =>
                      old[note.id] === (state === 'error')
                        ? old
                        : { ...old, [note.id]: state === 'error' },
                    )
                  }
                />
              ) : (
                <>
                  <strong className="note-row-title">
                    {note.title || t('未命名笔记', 'Untitled note')}
                  </strong>
                  <p className="note-row-summary">
                    {note.preview ||
                      t('还没有正文。留住此刻的想法。', 'A little space for your next thought.')}
                  </p>
                </>
              )}
              <div className="note-row-meta">
                <small>{displayDate(note.updated_at, language)}</small>
                <span title={status(note)}>
                  {note.inbox_state === 'pending' && <Inbox size={12} />}
                  {gallery && status(note)}
                </span>
              </div>
            </button>
            <div className="note-entry-actions">
              <PopupMenu
                label={t(`笔记操作：${note.title}`, `Note actions: ${note.title}`)}
                trigger={<Ellipsis size={16} />}
                placement="bottom-start"
                disabled={busy}
                items={[
                  ...(gallery
                    ? [
                        {
                          id: 'preview-retry',
                          label: t('重新读取预览', 'Reload preview'),
                          icon: <RefreshCw />,
                          onSelect: () =>
                            setPreviewRetries((old) => ({
                              ...old,
                              [note.id]: (old[note.id] ?? 0) + 1,
                            })),
                        },
                      ]
                    : []),
                  {
                    id: 'trash',
                    label: t('移入回收站', 'Move to trash'),
                    icon: <Trash2 />,
                    onSelect: () => onTrash(note.id),
                  },
                ]}
              />
            </div>
          </div>
        ))}
        {displayState === 'empty' && (
          <div className="list-empty">
            <FileText size={28} />
            <h2>
              {filter || tags.length
                ? t('没有匹配的笔记', 'No matching notes')
                : t('新的想法，从这里开始', 'Start with a thought')}
            </h2>
            <p>
              {filter || tags.length
                ? t('试试其他关键词，或清除筛选。', 'Try another keyword or clear your filters.')
                : t('记录一句话，也可以是一篇长文。', 'A sentence, a question, or a longer story.')}
            </p>
            {filter ? (
              <button className="text-button" onClick={() => onFilter('')}>
                {t('清除文本筛选', 'Clear text filter')}
              </button>
            ) : tags.length ? (
              <button className="text-button" onClick={() => onClearTag()}>
                {t('清除标签筛选', 'Clear tag filters')}
              </button>
            ) : (
              <button className="text-button" onClick={onCreate}>
                <Plus size={15} />
                {t('新建笔记', 'New note')}
              </button>
            )}
          </div>
        )}
      </div>
      <ListPagination list={notes} disabled={busy} />
    </section>
  );
}

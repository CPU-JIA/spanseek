import { useEffect, useId, useRef, useState } from 'react';
import { ArrowLeft, BookOpen, FileText, Highlighter, Pilcrow, Search, X } from 'lucide-react';
import type { ResolveLink, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { displayDate } from '../format';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { resolveReference } from './reference-context';
import { ReferenceNoteTarget } from './reference-note-target';
import { ReferenceSourceTarget } from './reference-source-target';
import {
  annotationReference,
  noteBlockReference,
  type ReferenceCandidate,
  type ReferenceDraft,
} from './reference-picker-model';

type CandidatePage = { items: ReferenceCandidate[]; next_cursor: string | null };
type Category = ReferenceCandidate['kind'];

/** The picker is a compact dialog; 50 rows keep its old request size as page one. */
const PICKER_PAGE_SIZE = 50;

export function ReferencePicker({
  identity,
  onSelect,
  onClose,
  mode = 'insert',
  excludeNoteId,
  excludeNoteIds = [],
  objectTypes = [],
}: {
  identity: WorkspaceIdentity;
  onSelect: (target: ResolveLink, label: string, alias?: string, embedded?: boolean) => void;
  onClose: () => void;
  mode?: 'insert' | 'note';
  excludeNoteId?: string;
  excludeNoteIds?: readonly string[];
  objectTypes?: readonly string[];
}) {
  const { t, language } = useI18n();
  const noteOnly = mode === 'note';
  const dialog = useRef<HTMLDialogElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const identityKey = `${identity.local_instance_id}/${identity.workspace_id}`;
  const active = useRef(identityKey);
  active.current = identityKey;
  const insertion = useRef(0);
  const [category, setCategory] = useState<Category>('note');
  const [query, setQuery] = useState('');
  const [searchQuery, setSearchQuery] = useState('');
  const [composing, setComposing] = useState(false);
  const composingRef = useRef(false);
  const returnFocus = useRef<HTMLElement | null>(null);
  const [archived, setArchived] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<ReferenceCandidate | null>(null);
  const [draft, setDraft] = useState<ReferenceDraft | null>(null);
  const [alias, setAlias] = useState('');
  const [embedded, setEmbedded] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const labels = {
    note: t('笔记', 'Notes'),
    block: t('内容块', 'Blocks'),
    source: t('资料与页面', 'Sources and pages'),
    annotation: t('摘录与批注', 'Excerpts and annotations'),
  };
  useEffect(() => {
    active.current = identityKey;
    returnFocus.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.showModal();
    requestAnimationFrame(() => search.current?.focus());
    return () => {
      active.current = '';
      const target = returnFocus.current;
      if (target?.isConnected) requestAnimationFrame(() => target.focus());
    };
  }, []);
  useEffect(() => {
    insertion.current++;
    setSelected(null);
    setDraft(null);
    setAlias('');
    setEmbedded(false);
    setSubmitting(false);
  }, [identityKey]);
  useEffect(() => {
    if (composing) return;
    const timer = setTimeout(() => setSearchQuery(query.trim()), 180);
    return () => clearTimeout(timer);
  }, [composing, query]);
  async function load(cursor: string | null, size: number): Promise<CandidatePage> {
    if (category === 'note') {
      const result = await call(
        {
          kind: 'notes_list',
          payload: {
            limit: size,
            cursor,
            trashed: false,
            filter: {
              inbox_state: null,
              tag_ids: [],
              object_types: noteOnly ? [...objectTypes] : [],
              archived,
            },
            sort: 'title_asc',
            query: searchQuery || null,
          },
        },
        'notes',
        identity,
      );
      return { ...result, items: result.items.map((value) => ({ kind: 'note', value })) };
    }
    if (category === 'block') {
      const result = await call(
        {
          kind: 'blocks_search',
          payload: {
            limit: size,
            cursor,
            query: searchQuery || null,
            note_id: null,
          },
        },
        'blocks',
        identity,
      );
      return { ...result, items: result.items.map((value) => ({ kind: 'block', value })) };
    }
    if (category === 'source') {
      const result = await call(
        {
          kind: 'sources_list',
          payload: {
            limit: size,
            cursor,
            trashed: false,
            filter: null,
            query: searchQuery || null,
          },
        },
        'sources',
        identity,
      );
      return { ...result, items: result.items.map((value) => ({ kind: 'source', value })) };
    }
    const result = await call(
      {
        kind: 'annotations_query',
        payload: {
          limit: size,
          cursor,
          query: searchQuery || null,
        },
      },
      'annotation_page',
      identity,
    );
    return { ...result, items: result.items.map((value) => ({ kind: 'annotation', value })) };
  }
  // One page of the cursor query at a time: the shared pager rebuilds page numbers
  // from the cursors it sees, so a page is never appended to the previous rows and
  // every filter or search term that redefines page one starts a new list.
  const scope = `${identityKey}/${category}/${archived ? 'archived' : 'active'}/${searchQuery}/${noteOnly ? [...objectTypes].sort().join(',') : ''}`;
  const list = usePagedList<ReferenceCandidate>(scope, load, PICKER_PAGE_SIZE);
  useEffect(() => setError(null), [scope]);
  function choose(item: ReferenceCandidate) {
    if (noteOnly && item.kind === 'note') {
      onSelect(
        { target_id: item.value.id, target_block_id: null, target_revision_id: null },
        item.value.title,
      );
      onClose();
      return;
    }
    insertion.current++;
    setSelected(item);
    setDraft(
      item.kind === 'annotation'
        ? annotationReference(item.value)
        : item.kind === 'block'
          ? noteBlockReference(item.value)
          : null,
    );
    setAlias('');
    setEmbedded(false);
    setError(null);
  }
  function back() {
    insertion.current++;
    setSelected(null);
    setDraft(null);
    setEmbedded(false);
    setError(null);
    requestAnimationFrame(() => search.current?.focus());
  }
  async function insert() {
    if (!draft || submitting) return;
    const scope = identityKey;
    const serial = ++insertion.current;
    setSubmitting(true);
    setError(null);
    try {
      const displayAlias = alias.trim();
      if (/[\u0000-\u001f\u007f]/.test(displayAlias))
        throw new Error(
          t('显示名称不能包含控制字符。', 'The display name cannot contain control characters.'),
        );
      // Validating an insertion must not accept a remembered answer.
      const latest = await resolveReference(identity, draft.target, { fresh: true });
      if (active.current !== scope || serial !== insertion.current) return;
      if (latest.state !== 'available' || latest.source_target?.locator_status === 'unavailable')
        throw new Error(
          t(
            '目标已变化或无法定位，请重新选择。',
            'The target changed or cannot be located. Select it again.',
          ),
        );
      onSelect(draft.target, latest.title || draft.label, displayAlias || undefined, embedded);
      onClose();
    } catch (cause) {
      if (active.current === scope && serial === insertion.current) setError(message(cause));
    } finally {
      if (active.current === scope && serial === insertion.current) setSubmitting(false);
    }
  }
  const excludedNoteIds = new Set(
    [excludeNoteId, ...excludeNoteIds].filter((id): id is string => Boolean(id)),
  );
  const visibleItems = (list.page?.items ?? []).filter(
    (item) => !(noteOnly && item.kind === 'note' && excludedNoteIds.has(item.value.id)),
  );
  return (
    <dialog
      ref={dialog}
      className="reference-picker"
      onCancel={(event) => {
        event.preventDefault();
        if (composingRef.current) return;
        onClose();
      }}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) {
          if (event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
          }
          return;
        }
        if (selected || !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
        const targets = [
          ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
            '.reference-target:not(:disabled)',
          ),
        ];
        if (!targets.length) return;
        const index = targets.indexOf(document.activeElement as HTMLButtonElement);
        let next: number;
        if (event.key === 'ArrowDown')
          next = index < 0 ? 0 : Math.min(index + 1, targets.length - 1);
        else if (event.key === 'ArrowUp') {
          if (index === 0) {
            event.preventDefault();
            search.current?.focus();
            return;
          }
          next = index < 0 ? targets.length - 1 : Math.max(index - 1, 0);
        } else if (event.key === 'Home') next = 0;
        else next = targets.length - 1;
        event.preventDefault();
        targets[next]?.focus();
      }}
      aria-labelledby={titleId}
    >
      <header>
        <h2 id={titleId}>
          {noteOnly ? t('选择关联笔记', 'Select linked note') : t('插入引用', 'Insert reference')}
        </h2>
        <button
          type="button"
          className="icon-button"
          aria-label={t('关闭', 'Close')}
          onClick={onClose}
        >
          <X size={18} />
        </button>
      </header>
      <div className="reference-picker-body">
        {selected ? (
          <>
            <button type="button" className="text-button" disabled={submitting} onClick={back}>
              <ArrowLeft size={15} /> {t('返回选择', 'Back to results')}
            </button>
            {selected.kind === 'note' ? (
              <ReferenceNoteTarget
                key={`${identityKey}/${selected.value.id}`}
                identity={identity}
                selected={selected.value}
                disabled={submitting}
                onChange={setDraft}
              />
            ) : selected.kind === 'source' ? (
              <ReferenceSourceTarget
                key={`${identityKey}/${selected.value.id}`}
                identity={identity}
                selected={selected.value}
                disabled={submitting}
                onChange={setDraft}
              />
            ) : selected.kind === 'block' ? (
              <>
                <h3>{selected.value.note_title}</h3>
                <p className="muted">
                  {t(
                    '引用当前版本中的稳定内容块；正文编辑后仍按块身份定位。',
                    'References a stable block in the current version and follows its identity as the note is edited.',
                  )}
                </p>
                <blockquote className="reference-selection-preview">
                  {selected.value.text || t('空内容块', 'Empty block')}
                </blockquote>
                <p className="reference-identity">
                  {selected.value.kind}
                  {selected.value.alias ? ` #${selected.value.alias}` : ''} ·{' '}
                  {selected.value.id.slice(-8)}
                </p>
                <label className="reference-embed">
                  <input
                    type="checkbox"
                    checked={embedded}
                    disabled={submitting}
                    onChange={(event) => setEmbedded(event.target.checked)}
                  />
                  <span>
                    <strong>{t('嵌入块内容', 'Embed block content')}</strong>
                    <small>
                      {t(
                        '在当前笔记中显示该块的实时内容，而不是只显示链接；标题块会连同所属章节显示。',
                        'Show this block inline instead of only linking it; a heading includes its section.',
                      )}
                    </small>
                  </span>
                </label>
              </>
            ) : (
              <>
                <h3>{selected.value.source_title}</h3>
                <p className="muted">
                  {t(
                    '固定摘录所在的原件版本，批注与原文双向关联。',
                    'Pinned to the original version containing this excerpt.',
                  )}
                </p>
                {selected.value.page_index !== null && (
                  <p>
                    {t(
                      `第 ${selected.value.page_index + 1} 页`,
                      `Page ${selected.value.page_index + 1}`,
                    )}
                  </p>
                )}
                <blockquote className="reference-selection-preview">
                  {selected.value.quote_preview || t('区域摘录', 'Area excerpt')}
                </blockquote>
                {selected.value.comment_preview && (
                  <p className="reference-annotation-preview">{selected.value.comment_preview}</p>
                )}
              </>
            )}
            <label className="reference-alias">
              <span>{t('显示名称（可选）', 'Display name (optional)')}</span>
              <input
                value={alias}
                maxLength={512}
                disabled={submitting}
                onChange={(event) => setAlias(event.target.value)}
                placeholder={t('留空时跟随目标标题', 'Leave empty to follow the target title')}
              />
            </label>
          </>
        ) : (
          <>
            {!noteOnly && (
              <div
                className="reference-categories"
                role="group"
                aria-label={t('引用类型', 'Reference type')}
              >
                {(['note', 'block', 'source', 'annotation'] as const).map((kind) => (
                  <button
                    type="button"
                    key={kind}
                    aria-pressed={category === kind}
                    onClick={() => setCategory(kind)}
                  >
                    {kind === 'note' ? (
                      <FileText size={15} />
                    ) : kind === 'block' ? (
                      <Pilcrow size={15} />
                    ) : kind === 'source' ? (
                      <BookOpen size={15} />
                    ) : (
                      <Highlighter size={15} />
                    )}
                    {labels[kind]}
                  </button>
                ))}
              </div>
            )}
            <label className="reference-search">
              <Search size={16} />
              <input
                ref={search}
                autoFocus
                aria-label={
                  noteOnly
                    ? t('搜索要关联的笔记', 'Search notes to link')
                    : t('搜索引用目标', 'Search reference targets')
                }
                placeholder={
                  category === 'annotation'
                    ? t('搜索摘录、批注或资料标题…', 'Search excerpts, comments, or source titles…')
                    : category === 'block'
                      ? t('搜索标题、正文或块锚…', 'Search titles, block text, or anchors…')
                      : noteOnly
                        ? t('搜索要关联的笔记标题…', 'Search note titles to link…')
                        : t('搜索标题…', 'Search titles…')
                }
                maxLength={512}
                value={query}
                aria-controls={`${titleId}-targets`}
                onChange={(event) => setQuery(event.target.value)}
                onCompositionStart={() => {
                  composingRef.current = true;
                  setComposing(true);
                }}
                onCompositionEnd={(event) => {
                  composingRef.current = false;
                  setComposing(false);
                  setSearchQuery(event.currentTarget.value.trim());
                }}
              />
            </label>
            {category === 'note' && (
              <label className="reference-archive">
                <input
                  type="checkbox"
                  checked={archived}
                  onChange={(event) => setArchived(event.target.checked)}
                />
                {t('查看已归档笔记', 'Show archived notes')}
              </label>
            )}
            <div
              id={`${titleId}-targets`}
              className="reference-target-list"
              aria-label={labels[category]}
              aria-busy={list.loading}
            >
              {visibleItems.map((item) => (
                <button
                  type="button"
                  key={`${item.kind}/${item.value.id}`}
                  className="reference-target"
                  disabled={
                    item.kind === 'block'
                      ? false
                      : item.value.trashed ||
                        (item.kind === 'annotation' && item.value.source_trashed)
                  }
                  onClick={() => choose(item)}
                  onKeyDown={(event) => {
                    if (
                      event.key !== 'Enter' ||
                      event.nativeEvent.isComposing ||
                      event.keyCode === 229
                    )
                      return;
                    event.preventDefault();
                    choose(item);
                  }}
                >
                  {item.kind === 'note' ? (
                    <FileText size={17} />
                  ) : item.kind === 'block' ? (
                    <Pilcrow size={17} />
                  ) : item.kind === 'source' ? (
                    <BookOpen size={17} />
                  ) : (
                    <Highlighter size={17} />
                  )}
                  <span>
                    <strong>
                      {item.kind === 'note'
                        ? item.value.title
                        : item.kind === 'block'
                          ? item.value.note_title
                          : item.kind === 'source'
                            ? item.value.display_title
                            : item.value.source_title}
                    </strong>
                    <small>
                      {item.kind === 'note'
                        ? item.value.preview || t('空笔记', 'Empty note')
                        : item.kind === 'block'
                          ? item.value.text || t('空内容块', 'Empty block')
                          : item.kind === 'source'
                            ? `${item.value.format.toUpperCase()} · ${displayDate(item.value.updated_at, language)}`
                            : item.value.quote_preview ||
                              item.value.comment_preview ||
                              t('区域摘录', 'Area excerpt')}
                    </small>
                    <small className="reference-identity">
                      {item.kind === 'note'
                        ? `${displayDate(item.value.updated_at, language)} · `
                        : item.kind === 'block'
                          ? `${item.value.kind}${item.value.alias ? ` #${item.value.alias}` : ''} · `
                          : item.kind === 'annotation' && item.value.page_index !== null
                            ? `${t(`第 ${item.value.page_index + 1} 页`, `Page ${item.value.page_index + 1}`)} · `
                            : ''}
                      {item.value.id.slice(-8)}
                    </small>
                  </span>
                </button>
              ))}
              {list.loading && <p role="status">{t('正在读取…', 'Loading…')}</p>}
              {!list.loading && list.page && !visibleItems.length && (
                <p className="muted">
                  {t('没有匹配的引用目标。', 'No matching reference targets.')}
                </p>
              )}
            </div>
            {list.page && (list.canNext || list.index > 1) && (
              <ListPagination list={list} className="reference-picker-pager" variant="inline" />
            )}
            <p className="reference-picker-hint">
              {noteOnly
                ? t(
                    '关联字段保存稳定笔记 ID 和当前标题；目标标题变化时仍按 ID 定位。最多添加 12 篇。',
                    'Relation fields keep a stable note ID and current title; the ID still resolves after a title change. Up to 12 notes.',
                  )
                : t(
                    '也可输入 [[笔记名#标题]]、[[笔记名#^块锚]] 或 ![[笔记名#^块锚]] 直接嵌入；同名目标请在此选择。',
                    'You can also type [[Note#Heading]], [[Note#^block]] or ![[Note#^block]] to embed. Choose here to distinguish identical titles.',
                  )}
            </p>
          </>
        )}
        {(error || list.error) && (
          <div className="inline-error" role="alert">
            <p>{error || list.error}</p>
            {!selected && (
              <button type="button" onClick={() => void list.loadFirst()}>
                {t('重试', 'Retry')}
              </button>
            )}
          </div>
        )}
      </div>
      {selected && (
        <footer>
          <button type="button" className="secondary-button" onClick={onClose}>
            {t('取消', 'Cancel')}
          </button>
          <button
            type="button"
            className="primary-button"
            disabled={!draft || submitting}
            onClick={() => void insert()}
          >
            {submitting
              ? t('正在确认…', 'Confirming…')
              : embedded
                ? t('插入嵌入', 'Insert embed')
                : t('插入引用', 'Insert reference')}
          </button>
        </footer>
      )}
    </dialog>
  );
}

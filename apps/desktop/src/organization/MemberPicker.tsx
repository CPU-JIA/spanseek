import { useEffect, useState } from 'react';
import { BookOpen, Check, FolderClosed, Quote, X } from 'lucide-react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { addableCount, memberCandidateRows } from './member-candidates';
import { NotePicker, QueryFeedback } from './OrganizationControls';
import './organization-pages.css';

type Kind = 'note' | 'source' | 'annotation' | 'collection';
type Candidate = {
  id: string;
  title: string;
  preview: string;
  pageIndex?: number | null;
  pendingText?: boolean;
};
export function MemberPicker({
  identity,
  selected,
  onChange,
  inCollection,
  disabled,
  onClose,
  context = 'collection',
}: {
  identity: WorkspaceIdentity;
  selected: string[];
  onChange: (ids: string[]) => void;
  /** Every member of the collection being edited, not only the page on screen. */
  inCollection: ReadonlySet<string>;
  disabled: boolean;
  onClose: () => void;
  context?: 'collection' | 'tag';
}) {
  const { t } = useI18n();
  const [kind, setKind] = useState<Kind>('note');
  return (
    <section
      className="org-member-picker"
      aria-label={t('添加已有内容', 'Add existing content')}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && !event.nativeEvent.isComposing && event.keyCode !== 229) {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="org-kind-tabs" role="group" aria-label={t('内容类型', 'Content type')}>
        {(context === 'tag'
          ? (['note', 'source', 'annotation', 'collection'] as const)
          : (['note', 'source', 'annotation'] as const)
        ).map((value) => (
          <button
            type="button"
            key={value}
            aria-pressed={kind === value}
            disabled={disabled}
            onClick={() => setKind(value)}
          >
            {value === 'note'
              ? t('笔记', 'Notes')
              : value === 'source'
                ? t('资料', 'Sources')
                : value === 'collection'
                  ? t('分类', 'Collections')
                  : t('摘录', 'Excerpts')}
          </button>
        ))}
      </div>
      {kind === 'note' ? (
        <NotePicker
          identity={identity}
          selected={selected}
          onChange={onChange}
          multiple
          inCollection={context === 'tag' ? undefined : inCollection}
          disabled={disabled}
          onClose={onClose}
        />
      ) : (
        <MaterialPicker
          key={kind}
          identity={identity}
          kind={kind}
          selected={selected}
          onChange={onChange}
          inCollection={inCollection}
          context={context}
          disabled={disabled}
          onClose={onClose}
        />
      )}
    </section>
  );
}

function MaterialPicker({
  identity,
  kind,
  selected,
  onChange,
  inCollection,
  disabled,
  onClose,
  context,
}: {
  identity: WorkspaceIdentity;
  kind: 'source' | 'annotation' | 'collection';
  selected: string[];
  onChange: (ids: string[]) => void;
  inCollection: ReadonlySet<string>;
  disabled: boolean;
  onClose: () => void;
  context: 'collection' | 'tag';
}) {
  const { t } = useI18n();
  const [text, setText] = useState('');
  const [query, setQuery] = useState('');
  const [archived, setArchived] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setQuery(text.trim()), 200);
    return () => clearTimeout(timer);
  }, [text]);
  const scope = JSON.stringify([
    identity.local_instance_id,
    identity.workspace_id,
    kind,
    query,
    archived,
  ]);
  const page = usePagedList<Candidate>(scope, async (cursor, size) => {
    if (kind === 'collection') {
      const result = await call(
        {
          kind: 'collections_list',
          payload: {
            limit: size,
            cursor,
            query: query || null,
            trashed: false,
            archived,
            pinned_only: false,
          },
        },
        'collections',
        identity,
      );
      return {
        ...result,
        items: result.items.map((item) => ({
          id: item.id,
          title: item.name,
          preview: t(`${item.member_count} 项内容`, `${item.member_count} items`),
        })),
      };
    }
    if (kind === 'source') {
      const result = await call(
        {
          kind: 'sources_list',
          payload: { limit: size, cursor, query: query || null, trashed: false, filter: null },
        },
        'sources',
        identity,
      );
      return {
        ...result,
        items: result.items.map((item) => ({
          id: item.id,
          title: item.display_title,
          preview: item.format.toUpperCase(),
          pendingText: !item.text_ready,
        })),
      };
    }
    const result = await call(
      { kind: 'annotations_query', payload: { limit: size, cursor, query: query || null } },
      'annotation_page',
      identity,
    );
    return {
      ...result,
      items: result.items.map((item) => ({
        id: item.id,
        title: item.quote_preview || item.comment_preview,
        preview: item.source_title,
        pageIndex: item.page_index,
      })),
    };
  });
  const rows = memberCandidateRows(page.page?.items ?? [], inCollection);
  const present = rows.length - addableCount(rows);
  return (
    <section
      className="org-picker"
      aria-label={
        kind === 'source'
          ? t('选择资料', 'Choose sources')
          : kind === 'collection'
            ? t('选择分类', 'Choose collections')
            : t('选择摘录', 'Choose excerpts')
      }
    >
      <header>
        <strong>
          {kind === 'source'
            ? t('从资料库添加', 'Add from your sources')
            : kind === 'collection'
              ? t('选择已有分类', 'Choose existing collections')
              : t('从全库摘录添加', 'Add from all excerpts')}
        </strong>
        <button
          type="button"
          className="icon-button"
          onClick={onClose}
          aria-label={t('关闭内容选择', 'Close content picker')}
        >
          <X size={16} />
        </button>
      </header>
      <label>
        {kind === 'source'
          ? t('按资料标题查找', 'Find by source title')
          : kind === 'collection'
            ? t('按分类名称查找', 'Find by collection name')
            : t('按摘录、评论或来源标题查找', 'Find by excerpt, comment, or source title')}
        <input
          className="ui-input"
          autoFocus
          placeholder={
            kind === 'collection'
              ? t('搜索分类', 'Search collections')
              : kind === 'source'
                ? t('搜索资料', 'Search sources')
                : t('搜索摘录', 'Search excerpts')
          }
          value={text}
          disabled={disabled}
          onChange={(event) => setText(event.target.value)}
        />
      </label>
      {kind === 'collection' && (
        <label className="org-check">
          <input
            type="checkbox"
            checked={archived}
            disabled={disabled}
            onChange={(event) => setArchived(event.target.checked)}
          />
          {t('查看已归档分类', 'Show archived collections')}
        </label>
      )}
      <div className="org-picker-options">
        {rows.map(({ item, inCollection: already }) => (
          <button
            type="button"
            key={item.id}
            disabled={
              disabled || already || (selected.length >= 100 && !selected.includes(item.id))
            }
            aria-pressed={selected.includes(item.id)}
            onClick={() =>
              onChange(
                selected.includes(item.id)
                  ? selected.filter((id) => id !== item.id)
                  : [...selected, item.id],
              )
            }
          >
            {kind === 'source' ? (
              <BookOpen size={16} />
            ) : kind === 'collection' ? (
              <FolderClosed size={16} />
            ) : (
              <Quote size={16} />
            )}
            <span>
              <strong>{item.title || t('未命名内容', 'Untitled content')}</strong>
              <small>
                {item.preview}
                {item.pageIndex != null &&
                  t(` · 第 ${item.pageIndex + 1} 页`, ` · Page ${item.pageIndex + 1}`)}
                {item.pendingText && t(' · 正文尚未就绪', ' · Text not ready')}
              </small>
            </span>
            {already && (
              <small className="org-member-state">{t('已在集合内', 'In this collection')}</small>
            )}
            {selected.includes(item.id) && <Check size={15} />}
          </button>
        ))}
      </div>
      {context === 'collection' && (
        <small className="org-picker-note">
          {t(
            `本页 ${rows.length} 项，其中 ${present} 项已在集合内`,
            `${rows.length} on this page, ${present} already in the collection`,
          )}
        </small>
      )}
      <QueryFeedback
        state={page}
        reload={() => void page.refresh()}
        pager={<ListPagination list={page} disabled={disabled} variant="inline" />}
        empty={t('没有匹配的内容。', 'No matching content.')}
      />
      <small>
        {t(
          `已选 ${selected.length} 项；每次最多 100 项`,
          `${selected.length} selected · Up to 100 at a time`,
        )}
      </small>
    </section>
  );
}

import { useEffect, useState } from 'react';
import { FolderClosed, LoaderCircle, Search } from 'lucide-react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { usePagedList, ListPagination } from '../layout/Pagination.tsx';

/** A collection is a list filter, never an intermediate landing page. */
export function CollectionNavigator({
  identity,
  selected,
  onSelect,
  onManage,
}: {
  identity: WorkspaceIdentity;
  selected: string | null;
  onSelect: (id: string | null) => void;
  onManage: () => void;
}) {
  const { t } = useI18n();
  const [expanded, setExpanded] = usePageNavigationState('sources.collections-expanded', true);
  const [query, setQuery] = usePageNavigationState('sources.collections-query', '');
  const [settledQuery, setSettledQuery] = useState(query);
  useEffect(() => {
    const timer = setTimeout(() => setSettledQuery(query), 180);
    return () => clearTimeout(timer);
  }, [query]);
  const list = usePagedList(
    JSON.stringify([identity.local_instance_id, identity.workspace_id, settledQuery.trim()]),
    (cursor, limit) =>
      call(
        {
          kind: 'collections_list',
          payload: {
            limit,
            cursor,
            trashed: false,
            archived: false,
            pinned_only: false,
            query: settledQuery.trim() || null,
          },
        },
        'collections',
        identity,
      ),
    20,
  );
  return (
    <aside className="source-collections" aria-label={t('资料集合', 'Document collections')}>
      <button
        className={`source-collection-item ${selected === null ? 'active' : ''}`}
        aria-pressed={selected === null}
        onClick={() => onSelect(null)}
      >
        <FolderClosed size={16} aria-hidden="true" />
        {t('全部资料', 'All documents')}
      </button>
      <details open={expanded} onToggle={(event) => setExpanded(event.currentTarget.open)}>
        <summary>{t('集合与分类', 'Collections & categories')}</summary>
        <label className="source-collection-search">
          <Search size={14} aria-hidden="true" />
          <input
            className="ui-input"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            aria-label={t('搜索集合', 'Search collections')}
            placeholder={t('搜索集合', 'Search collections')}
          />
        </label>
        {list.loading && (
          <p role="status">
            <LoaderCircle size={15} aria-hidden="true" />
            {t('正在读取…', 'Loading…')}
          </p>
        )}
        {list.error && (
          <div role="alert">
            <p>{list.error}</p>
            <button className="text-button" onClick={() => void list.refresh()}>
              {t('重试', 'Retry')}
            </button>
          </div>
        )}
        {!list.loading && list.page?.items.length === 0 && (
          <p className="muted">
            {query.trim()
              ? t('没有匹配的集合', 'No matching collections')
              : t('尚未创建集合', 'No collections yet')}
          </p>
        )}
        {list.page?.items.map((collection) => (
          <button
            key={collection.id}
            className={`source-collection-item ${selected === collection.id ? 'active' : ''}`}
            aria-pressed={selected === collection.id}
            title={collection.name}
            onClick={() => onSelect(collection.id)}
          >
            <FolderClosed size={15} aria-hidden="true" />
            <span>{collection.name}</span>
          </button>
        ))}
        {(list.canNext || list.canPrevious) && <ListPagination list={list} variant="inline" />}
      </details>
      <button className="text-button source-manage-collections" onClick={onManage}>
        {t('管理分类', 'Manage categories')}
      </button>
    </aside>
  );
}

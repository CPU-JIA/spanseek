import { useI18n } from '../i18n';
import { useEffect, useRef, useState } from 'react';
import { Hash, Search, X } from 'lucide-react';
import type { Tag, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { tagToneClass } from '../ui/tag-color';

export function TagFilter({
  identity,
  selected,
  onSelect,
  generation,
  showHeading = true,
}: {
  identity: WorkspaceIdentity;
  selected: Tag[];
  onSelect: (tags: Tag[]) => void;
  generation: number;
  showHeading?: boolean;
}) {
  const { t } = useI18n();
  const [query, setQuery] = useState('');
  const [applied, setApplied] = useState('');
  const [searching, setSearching] = useState(false);
  const searchButton = useRef<HTMLButtonElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  // Typing keeps the field responsive while the list waits for a pause, and a
  // new search is page 1 of a new list rather than more rows on the old one.
  useEffect(() => {
    const timer = setTimeout(() => setApplied(query), 150);
    return () => clearTimeout(timer);
  }, [query]);
  const scope = `${identity.local_instance_id}/${identity.workspace_id}/${generation}/${JSON.stringify(applied)}`;
  const tags = usePagedList<Tag>(scope, (cursor, size) =>
    call(
      { kind: 'tags_list', payload: { limit: size, cursor, query: applied || null } },
      'tags',
      identity,
    ),
  );
  useEffect(() => {
    if (searching) searchInput.current?.focus({ preventScroll: true });
  }, [searching]);
  return (
    <section className="tag-filter" aria-label={t('按标签筛选', 'Filter by tag')}>
      <div className="tag-filter-heading">
        {showHeading && <span>{t('标签', 'Tags')}</span>}
        <span className="spacer" />
        <button
          ref={searchButton}
          className="icon-button"
          aria-label={t('查找标签', 'Find tags')}
          title={t('查找标签', 'Find tags')}
          aria-expanded={searching}
          onClick={() => {
            setSearching(!searching);
            if (searching) setQuery('');
          }}
        >
          <Search size={13} />
        </button>
        {selected.length > 0 && (
          <button
            className="icon-button"
            aria-label={t('清除标签筛选', 'Clear tag filter')}
            onClick={() => onSelect([])}
          >
            <X size={14} />
          </button>
        )}
      </div>
      {searching && (
        <input
          ref={searchInput}
          aria-label={t('查找标签', 'Find tags')}
          placeholder={t('查找标签…', 'Find tags…')}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape' && !event.nativeEvent.isComposing && event.keyCode !== 229) {
              event.preventDefault();
              event.stopPropagation();
              setSearching(false);
              setQuery('');
              searchButton.current?.focus({ preventScroll: true });
            }
          }}
        />
      )}
      {selected.length > 1 && (
        <p className="tag-filter-match">{t('同时包含所选标签', 'Matches all selected tags')}</p>
      )}
      <div className="tag-filter-list">
        {tags.page?.items.map((tag) => (
          <button
            key={tag.id}
            className={`nav-item ${tagToneClass(tag.id)} ${selected.some((item) => item.id === tag.id) ? 'active' : ''}`}
            aria-pressed={selected.some((item) => item.id === tag.id)}
            onClick={() =>
              onSelect(
                selected.some((item) => item.id === tag.id)
                  ? selected.filter((item) => item.id !== tag.id)
                  : [...selected, tag],
              )
            }
          >
            <Hash size={15} />
            <span>{tag.display_name}</span>
          </button>
        ))}
      </div>
      {!tags.loading && !tags.error && tags.page && !tags.page.items.length && (
        <p className="muted">
          {applied
            ? t('没有匹配的标签', 'No matching tags')
            : t('在笔记中添加第一个标签', 'Add your first tag in a note')}
        </p>
      )}
      {tags.error && (
        <div role="alert">
          <p className="inline-error">{tags.error}</p>
          <button className="text-button" onClick={() => void tags.refresh()}>
            {t('重新读取', 'Reload')}
          </button>
        </div>
      )}
      <ListPagination list={tags} variant="inline" />
    </section>
  );
}

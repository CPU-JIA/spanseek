import { useEffect, useState } from 'react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { useCursorPage } from '../organization/page-hooks';
import { Combobox } from '../ui/Combobox';

export type SourceCollectionChoice = { id: string; name: string };
export function SourceCollectionPicker({
  identity,
  value,
  onChange,
  disabled,
  label,
  emptyLabel,
  generation = 0,
}: {
  identity: WorkspaceIdentity;
  value: SourceCollectionChoice | null;
  onChange: (value: SourceCollectionChoice | null) => void;
  disabled: boolean;
  label: string;
  emptyLabel?: string;
  generation?: number;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [debouncedQuery, setDebouncedQuery] = useState('');
  const [archived, setArchived] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQuery(query.trim()), 180);
    return () => clearTimeout(timer);
  }, [query]);
  const collections = useCursorPage<SourceCollectionChoice>(
    JSON.stringify([
      identity.local_instance_id,
      identity.workspace_id,
      open,
      debouncedQuery,
      archived,
      generation,
    ]),
    (cursor) =>
      open
        ? call(
            {
              kind: 'collections_list',
              payload: {
                limit: 30,
                cursor,
                trashed: false,
                archived,
                pinned_only: false,
                query: debouncedQuery || null,
              },
            },
            'collections',
            identity,
          )
        : Promise.resolve({ items: [], next_cursor: null }),
  );
  const choices = collections.page?.items ?? [];
  const namedChoices =
    value && !choices.some((item) => item.id === value.id) ? [value, ...choices] : choices;
  return (
    <div className="source-collection-picker">
      <label className="source-field-label">
        <span>{label}</span>
        <Combobox
          aria-label={label}
          value={value?.id ?? ''}
          onValueChange={(id) => onChange(namedChoices.find((item) => item.id === id) ?? null)}
          options={[
            { value: '', label: emptyLabel ?? t('不指定集合', 'No collection') },
            ...namedChoices.map((item) => ({ value: item.id, label: item.name })),
          ]}
          disabled={disabled}
          filterOptions={false}
          onQueryChange={setQuery}
          onOpenChange={(next) => {
            setOpen(next);
            if (next) {
              setQuery('');
              setDebouncedQuery('');
            }
          }}
          loading={open && (collections.loading || query.trim() !== debouncedQuery)}
          placeholder={t('搜索集合…', 'Search collections…')}
          emptyText={t('没有匹配集合', 'No matching collections')}
          footer={
            <div className="source-choice-footer">
              <label className="source-check-label">
                <input
                  type="checkbox"
                  disabled={disabled}
                  checked={archived}
                  onChange={(event) => setArchived(event.target.checked)}
                />
                {t('查看已归档集合', 'Show archived collections')}
              </label>
              {collections.error && <p role="alert">{collections.error}</p>}
              {collections.error && (
                <button
                  type="button"
                  disabled={disabled || collections.loading}
                  onClick={() => void collections.reload()}
                >
                  {t('重试', 'Retry')}
                </button>
              )}
              {!collections.loading && !collections.error && !choices.length && (
                <p>
                  {t(
                    '没有匹配集合，可在集合页创建。',
                    'No matching collections. Create one on the Collections page.',
                  )}
                </p>
              )}
              {collections.page?.next_cursor && (
                <button
                  type="button"
                  disabled={disabled || collections.loading}
                  onClick={() => void collections.more()}
                >
                  {t('加载更多集合', 'Load more collections')}
                </button>
              )}
            </div>
          }
        />
      </label>
    </div>
  );
}

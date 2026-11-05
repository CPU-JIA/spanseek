import { useEffect, useMemo, useState } from 'react';
import type { KnowledgeObject, KnowledgeObjectFilter, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { SourceLibraryDialog } from '../sources/SourceImportDialog';
import { KnowledgeBatchBar } from './KnowledgeBatchBar';

export function SourceImportOrganizerDialog({
  identity,
  sourceIds,
  onClose,
  onChanged,
}: {
  identity: WorkspaceIdentity;
  sourceIds: string[];
  onClose: () => void;
  onChanged: () => void;
}) {
  const { t } = useI18n();
  const [items, setItems] = useState<KnowledgeObject[]>([]);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [generation, setGeneration] = useState(0);
  const ids = useMemo(() => [...new Set(sourceIds)].slice(0, 500), [sourceIds]);
  const scope = ids.join('|');

  useEffect(() => {
    let live = true;
    setLoading(true);
    setError(null);
    async function load() {
      const pages: KnowledgeObject[] = [];
      for (let offset = 0; offset < ids.length; offset += 100) {
        const chunk = ids.slice(offset, offset + 100);
        const filter: KnowledgeObjectFilter = {
          kinds: [],
          collection_id: null,
          uncategorized: false,
          tag_ids: [],
          object_ids: chunk,
          object_types: [],
          inbox_state: null,
          archived: false,
          trashed: false,
        };
        const page = await call(
          {
            kind: 'knowledge_objects_list',
            payload: { limit: chunk.length, cursor: null, query: null, filter },
          },
          'knowledge_objects',
          identity,
        );
        pages.push(...page.items);
      }
      if (!live) return;
      const byId = new Map(pages.map((item) => [item.object.id, item]));
      const ordered = ids.flatMap((id) => {
        const item = byId.get(id);
        return item ? [item] : [];
      });
      setItems(ordered);
      setSelectedIds(ordered.map((item) => item.object.id));
    }
    void load()
      .catch((caught) => {
        if (live) setError(message(caught));
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, [identity, scope, generation]);

  const selected = items.filter((item) => selectedIds.includes(item.object.id));
  return (
    <SourceLibraryDialog
      title={t('整理本次导入', 'Organize this import')}
      onClose={onClose}
      className="source-organizer-dialog"
    >
      <div className="source-import-organizer">
        <p className="muted">
          {t(
            '勾选要调整的资料。可先选中一部分加入分类或标签，再选择其余资料应用不同整理结果。',
            'Select the documents to adjust. Apply a category or tag to one subset, then select the rest for a different result.',
          )}
        </p>
        {loading && (
          <p className="muted" role="status">
            {t('正在读取本次导入…', 'Loading this import…')}
          </p>
        )}
        {error && (
          <p className="source-dialog-error" role="alert">
            {error}
            <button type="button" onClick={() => setGeneration((value) => value + 1)}>
              {t('重试', 'Retry')}
            </button>
          </p>
        )}
        {!loading && !error && !items.length && (
          <p className="muted">
            {t(
              '导入对象尚不可用，请稍后在收件箱继续整理。',
              'The imported objects are not ready yet. Continue in the inbox shortly.',
            )}
          </p>
        )}
        {items.length > 0 && (
          <ul className="source-import-organizer-list">
            {items.map((item) => {
              const checked = selectedIds.includes(item.object.id);
              return (
                <li key={item.object.id}>
                  <label>
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={() =>
                        setSelectedIds((current) =>
                          checked
                            ? current.filter((id) => id !== item.object.id)
                            : [...current, item.object.id],
                        )
                      }
                    />
                    <span className="source-import-organizer-copy">
                      <strong>{item.title}</strong>
                      <small>
                        {item.collections.length
                          ? item.collections.map((collection) => collection.name).join(' · ')
                          : t('未分类', 'Uncategorized')}
                        {item.object.tags.length
                          ? ` · ${item.object.tags.map((tag) => `#${tag.display_name}`).join(' ')}`
                          : ''}
                      </small>
                    </span>
                    <span
                      className={`knowledge-chip ${item.object.inbox_state === 'pending' ? 'is-pending' : ''}`}
                    >
                      {item.object.inbox_state === 'pending'
                        ? t('待整理', 'Inbox')
                        : item.object.inbox_state === 'snoozed'
                          ? t('稍后', 'Later')
                          : t('已整理', 'Organized')}
                    </span>
                  </label>
                </li>
              );
            })}
          </ul>
        )}
        {selected.length > 0 && (
          <KnowledgeBatchBar
            identity={identity}
            selected={selected}
            onClear={() => setSelectedIds([])}
            onChanged={() => {
              setSelectedIds([]);
              setGeneration((value) => value + 1);
              onChanged();
            }}
          />
        )}
        <footer className="source-library-dialog-footer">
          <button type="button" onClick={onClose}>
            {t('完成', 'Done')}
          </button>
        </footer>
      </div>
    </SourceLibraryDialog>
  );
}

import { useState } from 'react';
import { FolderPlus, LoaderCircle, Tag, Trash2, X } from 'lucide-react';
import type { InboxState, KnowledgeObject, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import { useKnowledgeTaskNavigation } from '../inference/knowledge-task-navigation';
import {
  SourceCollectionPicker,
  type SourceCollectionChoice,
} from '../sources/SourceCollectionPicker';

type BatchAction = 'category' | 'tag' | 'pending' | 'snoozed' | 'processed' | 'trash';
type BatchResult = { id: string; title: string; status: 'done' | 'failed'; error?: string };

export function KnowledgeBatchBar({
  identity,
  selected,
  onClear,
  onChanged,
  beforeTask,
}: {
  identity: WorkspaceIdentity;
  selected: KnowledgeObject[];
  onClear: () => void;
  onChanged: () => void;
  beforeTask?: () => void;
}) {
  const { t } = useI18n();
  const taskNavigation = useKnowledgeTaskNavigation();
  const [collection, setCollection] = useState<SourceCollectionChoice | null>(null);
  const [tagName, setTagName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [results, setResults] = useState<BatchResult[] | null>(null);
  const [confirmTrash, setConfirmTrash] = useState(false);
  const [lastAction, setLastAction] = useState<BatchAction | null>(null);
  const [retryIds, setRetryIds] = useState<string[]>([]);

  async function apply(action: BatchAction, targets: KnowledgeObject[] = selected) {
    if (!targets.length || busy) return;
    setBusy(true);
    setLastAction(action);
    setRetryIds([]);
    setError(null);
    setResults(null);
    const next: BatchResult[] = [];
    try {
      if (action === 'category') {
        if (!collection) throw new Error(t('请选择一个分类。', 'Choose a category.'));
        const detail = await call(
          { kind: 'collections_get', payload: { collection_id: collection.id } },
          'collection',
          identity,
        );
        await call(
          {
            kind: 'collections_add_members',
            payload: {
              command_id: crypto.randomUUID(),
              request: {
                collection_id: detail.id,
                base_row_version: detail.metadata.row_version,
                member_ids: targets.map((item) => item.object.id),
              },
            },
          },
          'collection_commit',
          identity,
        );
        for (const item of targets) {
          try {
            const commit = await call(
              {
                kind: 'objects_set_inbox_state',
                payload: {
                  command_id: crypto.randomUUID(),
                  request: {
                    object_id: item.object.id,
                    base_row_version: item.object.row_version,
                    target: 'processed' as InboxState,
                  },
                },
              },
              'metadata_commit',
              identity,
            );
            next.push({
              id: item.object.id,
              title: item.title,
              status: commit.objects[0] ? 'done' : 'failed',
            });
          } catch (caught) {
            next.push({
              id: item.object.id,
              title: item.title,
              status: 'failed',
              error: message(caught),
            });
          }
        }
      } else if (action === 'tag') {
        const name = tagName.trim();
        if (!name) throw new Error(t('请输入标签名。', 'Enter a tag name.'));
        for (const item of targets) {
          try {
            const commit = await call(
              {
                kind: 'tags_assign',
                payload: {
                  command_id: crypto.randomUUID(),
                  request: {
                    targets: [
                      { object_id: item.object.id, base_row_version: item.object.row_version },
                    ],
                    tag: { kind: 'name', name },
                  },
                },
              },
              'metadata_commit',
              identity,
            );
            const done = commit.objects.some((object) => object.id === item.object.id);
            next.push({
              id: item.object.id,
              title: item.title,
              status: done ? 'done' : 'failed',
              error: done
                ? undefined
                : t(
                    '服务未返回该对象，请刷新后重试。',
                    'The service did not return this object; refresh and retry.',
                  ),
            });
          } catch (caught) {
            next.push({
              id: item.object.id,
              title: item.title,
              status: 'failed',
              error: message(caught),
            });
          }
        }
      } else {
        const target = action === 'trash' ? null : (action as InboxState);
        for (const item of targets) {
          try {
            const command = target
              ? {
                  kind: 'objects_set_inbox_state' as const,
                  payload: {
                    command_id: crypto.randomUUID(),
                    request: {
                      object_id: item.object.id,
                      base_row_version: item.object.row_version,
                      target,
                    },
                  },
                }
              : {
                  kind: 'objects_set_trashed' as const,
                  payload: {
                    command_id: crypto.randomUUID(),
                    request: {
                      object_id: item.object.id,
                      base_row_version: item.object.row_version,
                      value: true,
                    },
                  },
                };
            const commit = await call(command, 'metadata_commit', identity);
            next.push({
              id: item.object.id,
              title: item.title,
              status: commit.objects[0] ? 'done' : 'failed',
            });
          } catch (caught) {
            next.push({
              id: item.object.id,
              title: item.title,
              status: 'failed',
              error: message(caught),
            });
          }
        }
      }
      setResults(next);
      setRetryIds(next.filter((item) => item.status === 'failed').map((item) => item.id));
      onChanged();
      if (next.every((item) => item.status === 'done')) {
        setLastAction(null);
        onClear();
      }
    } catch (caught) {
      setError(message(caught));
    } finally {
      setBusy(false);
      setConfirmTrash(false);
    }
  }

  const failed = results?.filter((item) => item.status === 'failed').length ?? 0;

  function retryFailed() {
    if (!lastAction || !retryIds.length) return;
    const targets = selected.filter((item) => retryIds.includes(item.object.id));
    if (!targets.length) {
      setError(
        t(
          '失败项已不在当前列表中，请刷新后重新选择。',
          'The failed objects are no longer in this list. Refresh and select them again.',
        ),
      );
      return;
    }
    void apply(lastAction, targets);
  }
  return (
    <section
      className="knowledge-batch"
      aria-label={t('批量整理', 'Batch organization')}
      aria-busy={busy}
    >
      <header>
        <strong>{t(`已选择 ${selected.length} 个对象`, `${selected.length} selected`)}</strong>
        <button
          type="button"
          className="icon-button"
          aria-label={t('清除选择', 'Clear selection')}
          onClick={onClear}
          disabled={busy}
        >
          <X size={15} />
        </button>
      </header>
      <div className="knowledge-batch-actions">
        {taskNavigation && (
          <button
            className="secondary-button"
            aria-label={t('用所选内容发起任务', 'Start task with selection')}
            disabled={busy}
            onClick={() => {
              beforeTask?.();
              taskNavigation.start(
                selected.map((item) => ({
                  object_id: item.object.id,
                  content_revision:
                    item.note?.revision_id ??
                    item.source?.current_version_id ??
                    item.annotation?.source_version_id ??
                    null,
                })),
              );
            }}
          >
            {t('发起任务', 'Create task')}
          </button>
        )}
        <SourceCollectionPicker
          identity={identity}
          value={collection}
          onChange={setCollection}
          disabled={busy}
          label={t('加入分类', 'Add to category')}
          emptyLabel={t('选择分类…', 'Choose a category…')}
        />
        <button
          type="button"
          className="secondary-button"
          disabled={busy || !collection}
          onClick={() => void apply('category')}
        >
          <FolderPlus size={14} />
          {t('加入并完成', 'Add and organize')}
        </button>
        <label className="knowledge-batch-tag">
          <Tag size={13} />
          <input
            value={tagName}
            onChange={(event) => setTagName(event.target.value)}
            placeholder={t('输入标签名…', 'Tag name…')}
            aria-label={t('批量添加标签', 'Add a tag to selected objects')}
            disabled={busy}
          />
          <button
            type="button"
            className="text-button"
            disabled={busy || !tagName.trim()}
            onClick={() => void apply('tag')}
          >
            {t('添加', 'Add')}
          </button>
        </label>
        <div
          className="knowledge-batch-segments"
          role="group"
          aria-label={t('批量整理状态', 'Batch organization state')}
        >
          <button type="button" disabled={busy} onClick={() => void apply('pending')}>
            {t('待整理', 'Inbox')}
          </button>
          <button type="button" disabled={busy} onClick={() => void apply('snoozed')}>
            {t('稍后', 'Later')}
          </button>
          <button type="button" disabled={busy} onClick={() => void apply('processed')}>
            {t('完成整理', 'Organized')}
          </button>
        </div>
        <button
          type="button"
          className="text-button knowledge-batch-trash"
          disabled={busy}
          onClick={() => setConfirmTrash(true)}
        >
          <Trash2 size={14} />
          {t('回收', 'Trash')}
        </button>
        {busy && <LoaderCircle size={15} className="spin" aria-label={t('正在整理', 'Working')} />}
      </div>
      {error && (
        <p className="knowledge-context-error" role="alert">
          {error}
        </p>
      )}
      {results && (
        <div className="knowledge-batch-results" role="status">
          <span>
            {failed
              ? t(
                  `${(results?.length ?? 0) - failed} 个完成，${failed} 个失败`,
                  `${(results?.length ?? 0) - failed} done, ${failed} failed`,
                )
              : t('全部完成', 'All done')}
          </span>
          {failed > 0 && (
            <button type="button" className="text-button" disabled={busy} onClick={retryFailed}>
              {t('重试失败项', 'Retry failures')}
            </button>
          )}
          {failed > 0 && (
            <details open>
              <summary>{t('查看失败项', 'View failures')}</summary>
              <ul>
                {results
                  .filter((item) => item.status === 'failed')
                  .map((item) => (
                    <li key={item.id}>
                      <strong>{item.title}</strong>
                      <small>{item.error}</small>
                    </li>
                  ))}
              </ul>
            </details>
          )}
        </div>
      )}
      {confirmTrash && (
        <ActionConfirmDialog
          title={t(`回收 ${selected.length} 个对象？`, `Move ${selected.length} objects to trash?`)}
          description={t(
            '对象会进入回收站，整理信息仍可恢复。',
            'The objects move to trash; organization metadata remains recoverable.',
          )}
          confirmLabel={t('移入回收站', 'Move to trash')}
          tone="danger"
          onCancel={() => setConfirmTrash(false)}
          onConfirm={() => void apply('trash')}
        />
      )}
    </section>
  );
}

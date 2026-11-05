import { useEffect, useState } from 'react';
import type { KnowledgeTaskOriginsPage, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { Pagination } from '../layout/Pagination.tsx';
import { useKnowledgeTaskNavigation } from './knowledge-task-navigation';
import './knowledge-tasks.css';

export function TaskOriginPanel({
  identity,
  noteId,
  revisionId,
}: {
  identity: WorkspaceIdentity;
  noteId: string;
  revisionId: string;
}) {
  const { t, language } = useI18n();
  const navigation = useKnowledgeTaskNavigation();
  const [data, setData] = useState<KnowledgeTaskOriginsPage | null>(null);
  const [page, setPage] = useState(1);
  const [size, setSize] = useState(10);
  const [reload, setReload] = useState(0);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    if (!navigation) return;
    let live = true;
    setLoading(true);
    setError('');
    void call(
      { kind: 'knowledge_task_origins', payload: { note_id: noteId, page, limit: size } },
      'knowledge_task_origins',
      identity,
    )
      .then((value) => {
        if (live) {
          setData(value);
          setPage(value.page);
        }
      })
      .catch((error) => {
        if (live) setError(message(error));
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, noteId, revisionId, page, size, reload]);
  if (!navigation || (!error && !data?.total)) return null;
  return (
    <details className="task-origin-panel">
      <summary>
        {t('生成来源与任务', 'Generation sources and tasks')}
        {data?.total ? ` · ${data.total}` : ''}
      </summary>
      {error && (
        <p role="alert">
          {error}
          <button onClick={() => setReload((n) => n + 1)}>{t('重试读取', 'Reload')}</button>
        </p>
      )}
      <ul aria-busy={loading}>
        {data?.items.map((item) => (
          <li key={`${item.task_id}/${item.applied_revision_id}`}>
            <button className="text-button" onClick={() => navigation.open(item.task_id)}>
              {item.task_title}
            </button>
            <span>
              {t(`${item.materials_count} 个材料片段`, `${item.materials_count} material excerpts`)}{' '}
              · {displayDate(item.task_created_at, language)}
            </span>
            <small>
              {item.applied_revision_id === revisionId
                ? t('当前修订由此任务保存', 'This task saved the current revision')
                : t(
                    '保留当时保存的修订；正文后来已有变化',
                    'The saved revision is retained; this note has changed since',
                  )}
            </small>
          </li>
        ))}
      </ul>
      {data && data.total > size && (
        <Pagination
          index={page}
          size={size}
          count={data.items.length}
          total={data.total}
          loading={loading}
          canPrevious={page > 1}
          canNext={page * size < data.total}
          onSize={(n) => {
            setSize(n);
            setPage(1);
          }}
          onPrevious={() => setPage((n) => n - 1)}
          onNext={() => setPage((n) => n + 1)}
          onGoTo={setPage}
        />
      )}
    </details>
  );
}

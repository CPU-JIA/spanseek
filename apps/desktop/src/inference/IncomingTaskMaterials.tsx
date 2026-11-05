import { useEffect, useRef, useState } from 'react';
import type { KnowledgeTaskMaterials, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { SourceLibraryDialog } from '../sources/SourceImportDialog';
import { inferenceErrorMessage } from './inference-errors';
import type { TaskMaterialRequest } from './knowledge-task-navigation';
import {
  emptyTaskDraft,
  materialCoverageText,
  mergeTaskMaterials,
  type TaskDraft,
} from './knowledge-task-state';

export function IncomingTaskMaterials({
  identity,
  request,
  value,
  unreadable,
  save,
  onHandled,
}: {
  identity: WorkspaceIdentity;
  request: TaskMaterialRequest;
  value: TaskDraft;
  unreadable: boolean;
  save: (value: TaskDraft, replace?: boolean) => boolean;
  onHandled: (id: string) => void;
}) {
  const { t } = useI18n();
  const [data, setData] = useState<KnowledgeTaskMaterials | null>(null);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const latest = useRef({ value, unreadable, save, onHandled });
  latest.current = { value, unreadable, save, onHandled };
  function merged(data: KnowledgeTaskMaterials, draft: TaskDraft) {
    return mergeTaskMaterials(
      draft,
      data.materials,
      data.coverage.map((c) => c.object_id),
      data.coverage.filter((c) => c.status !== 'complete'),
    );
  }
  useEffect(() => {
    if (
      request.identity.local_instance_id !== identity.local_instance_id ||
      request.identity.workspace_id !== identity.workspace_id
    ) {
      latest.current.onHandled(request.id);
      return;
    }
    let live = true;
    setData(null);
    setError('');
    void call(
      { kind: 'knowledge_task_materials', payload: { targets: request.targets } },
      'knowledge_task_materials',
      identity,
    )
      .then((data) => {
        if (!live) return;
        const draft = latest.current;
        if (
          !draft.unreadable &&
          !draft.value.title &&
          !draft.value.instructions &&
          !draft.value.materials.length
        ) {
          if (draft.save(merged(data, draft.value))) draft.onHandled(request.id);
          else setData(data);
        } else setData(data);
      })
      .catch((failure) => {
        if (live) setError(inferenceErrorMessage(failure, t));
      });
    return () => {
      live = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, request.id, retry]);
  if (!data)
    return (
      <div className="task-material-loading" role={error ? 'alert' : 'status'}>
        <p>{error || t('正在读取所选材料…', 'Loading selected materials…')}</p>
        {error && (
          <button className="secondary-button" onClick={() => setRetry((n) => n + 1)}>
            {t('重新读取', 'Reload')}
          </button>
        )}
        <button className="text-button" onClick={() => onHandled(request.id)}>
          {t('取消加入材料', 'Cancel adding materials')}
        </button>
      </div>
    );
  const addition = merged(data, value);
  const fits =
    addition.materials.length <= 32 &&
    addition.materials.reduce((n, e) => n + new TextEncoder().encode(e.text).length, 0) <=
      256 * 1024;
  function accept(replace: boolean) {
    if (save(merged(data!, replace ? emptyTaskDraft() : value), replace)) onHandled(request.id);
  }
  return (
    <SourceLibraryDialog
      title={t('加入所选材料', 'Add selected materials')}
      className="task-material-dialog"
      onClose={() => onHandled(request.id)}
    >
      <div className="task-template-body">
        <p>
          {t(
            '保留当前任务要求；相同来源的旧片段会更新为这次选择。也可以新建任务并替换当前草稿。',
            'Keep the current instructions and replace excerpts from the same sources with this selection, or start a new task and replace the current draft.',
          )}
        </p>
        <ul className="task-coverage-list">
          {data.coverage.map((item) => (
            <li key={item.object_id}>
              <strong>{item.title || t('所选条目', 'Selected item')}</strong>
              <span>{materialCoverageText(item, t)}</span>
            </li>
          ))}
        </ul>
        {!fits && (
          <p role="status">
            {t(
              '合并后超过任务容量，请移除部分旧材料，或新建任务。',
              'The combined materials exceed task capacity. Remove some previous materials or start a new task.',
            )}
          </p>
        )}
      </div>
      <footer className="task-template-footer">
        <button className="secondary-button" onClick={() => accept(true)}>
          {t('新建任务，替换草稿', 'Replace draft with new task')}
        </button>
        <button
          className="primary-button"
          disabled={unreadable || !fits}
          onClick={() => accept(false)}
        >
          {t('加入当前草稿', 'Add to current draft')}
        </button>
      </footer>
    </SourceLibraryDialog>
  );
}

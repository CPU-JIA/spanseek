import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Copy, Pencil, Plus, Trash2 } from 'lucide-react';
import type {
  KnowledgeTaskTemplate,
  KnowledgeTaskTemplatesPage,
  SaveKnowledgeTaskTemplate,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, ServiceError } from '../api';
import { useI18n } from '../i18n';
import { Pagination } from '../layout/Pagination.tsx';
import { SourceLibraryDialog } from '../sources/SourceImportDialog';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import { useInferenceAction } from './useInferenceAction';
import { inferenceErrorMessage } from './inference-errors';
import '../sources/sources.css';

const TEMPLATE_NAME_LIMIT = 100;
const blank = (): SaveKnowledgeTaskTemplate => ({
  id: null,
  base_row_version: null,
  name: '',
  description: '',
  instructions: '',
});
function copyName(name: string, suffix: string) {
  return [...name].slice(0, TEMPLATE_NAME_LIMIT - [...suffix].length).join('') + suffix;
}
export function TaskTemplatesDialog({
  identity,
  onUse,
  onClose,
}: {
  identity: WorkspaceIdentity;
  onUse: (template: KnowledgeTaskTemplate) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(1);
  const [size, setSize] = useState(10);
  const [data, setData] = useState<KnowledgeTaskTemplatesPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [reload, setReload] = useState(0);
  const [error, setError] = useState('');
  const [draft, setDraft] = useState<SaveKnowledgeTaskTemplate | null>(null);
  const body = useRef<HTMLDivElement>(null);
  const errorNotice = useRef<HTMLParagraphElement>(null);
  const listScroll = useRef(0);
  const original = useRef('');
  const [discard, setDiscard] = useState<(() => void) | null>(null);
  const [deleting, setDeleting] = useState<KnowledgeTaskTemplate | null>(null);
  const live = useRef(true);
  const actions = useInferenceAction(identity, 'knowledge-templates', (failure) => {
    if (failure instanceof ServiceError && failure.details.code === 'metadata_conflict')
      return draft
        ? t(
            '模板已有更新，本次修改未写入。请重新读取后核对，或将当前修改另存为新模板。',
            'The template has changed; this operation was not saved. Reload to review it, or save your changes as a new template.',
          )
        : t(
            '模板已有更新，本次删除未执行。请重新读取后再决定是否删除。',
            'The template has changed and was not deleted. Reload it before deciding whether to delete it.',
          );
    if (failure instanceof ServiceError && failure.details.code === 'not_found')
      return draft
        ? t(
            '模板已被删除。可以重新读取列表，或将当前修改另存为新模板。',
            'This template was deleted. Reload the list, or save your changes as a new template.',
          )
        : t('模板已被删除，请重新读取列表。', 'This template was deleted. Reload the list.');
    return inferenceErrorMessage(failure, t);
  });
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  useEffect(() => {
    let current = true;
    setLoading(true);
    setError('');
    const timer = setTimeout(
      () => {
        void call(
          { kind: 'knowledge_task_templates_page', payload: { page, limit: size, query } },
          'knowledge_task_templates_page',
          identity,
        )
          .then((result) => {
            if (current) {
              setData(result);
              setPage(result.page);
            }
          })
          .catch((failure) => {
            if (current) setError(inferenceErrorMessage(failure, t));
          })
          .finally(() => {
            if (current) setLoading(false);
          });
      },
      query ? 180 : 0,
    );
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [page, size, query, reload, identity.local_instance_id, identity.workspace_id]);
  useLayoutEffect(() => {
    if (body.current) body.current.scrollTop = draft ? 0 : listScroll.current;
  }, [!!draft, data]);
  useLayoutEffect(() => {
    if (error || actions.error) errorNotice.current?.scrollIntoView({ block: 'nearest' });
  }, [error, actions.error]);
  function edit(value: SaveKnowledgeTaskTemplate) {
    if (!draft) listScroll.current = body.current?.scrollTop ?? 0;
    actions.clearError();
    setError('');
    original.current = JSON.stringify(value);
    setDraft(value);
  }
  function leave(next: () => void) {
    if (actions.busy || !actions.ready) return;
    if (actions.pending) {
      next();
      return;
    }
    if (draft && JSON.stringify(draft) !== original.current) setDiscard(() => next);
    else next();
  }
  async function save() {
    if (!draft) return;
    const result = await actions.execute(
      {
        kind: 'knowledge_task_templates_save',
        payload: { command_id: crypto.randomUUID(), request: draft },
      },
      'knowledge_task_template_mutation',
    );
    if (result && live.current) {
      setDraft(null);
      setReload((n) => n + 1);
    }
  }
  async function remove(template: KnowledgeTaskTemplate) {
    setDeleting(null);
    const result = await actions.execute(
      {
        kind: 'knowledge_task_templates_delete',
        payload: {
          command_id: crypto.randomUUID(),
          request: { id: template.id, base_row_version: template.row_version },
        },
      },
      'knowledge_task_template_mutation',
    );
    if (result && live.current) setReload((n) => n + 1);
  }
  async function recover() {
    const result = await actions.retry<'knowledge_task_template_mutation'>();
    if (result && live.current) {
      setDraft(null);
      setReload((n) => n + 1);
    }
  }
  async function reread() {
    if (!draft?.id) return;
    try {
      const value = await call(
        { kind: 'knowledge_task_templates_get', payload: { template_id: draft.id } },
        'knowledge_task_template',
        identity,
      );
      if (live.current)
        edit({
          id: value.id,
          base_row_version: value.row_version,
          name: value.name,
          description: value.description,
          instructions: value.instructions,
        });
    } catch (failure) {
      if (live.current) setError(inferenceErrorMessage(failure, t));
    }
  }
  return (
    <SourceLibraryDialog
      title={draft ? t('编辑任务模板', 'Edit task template') : t('任务模板', 'Task templates')}
      className="task-template-dialog"
      onClose={() => leave(onClose)}
    >
      <div className="task-template-body" ref={body}>
        {(error || actions.error) && (
          <p role="alert" ref={errorNotice}>
            {error || actions.error}
          </p>
        )}
        {actions.pending && (
          <button
            type="button"
            className="secondary-button"
            disabled={actions.busy}
            onClick={() => void recover()}
          >
            {t('确认上次操作', 'Recover previous action')}
          </button>
        )}
        {draft?.id && actions.error && (
          <div className="task-row-actions">
            <button
              type="button"
              disabled={actions.locked}
              onClick={() => leave(() => void reread())}
            >
              {t('重新读取模板', 'Reload template')}
            </button>
            <button
              type="button"
              disabled={actions.locked}
              onClick={() => {
                edit({
                  ...draft,
                  id: null,
                  base_row_version: null,
                  name: copyName(draft.name, t(' 副本', ' copy')),
                });
                original.current = JSON.stringify(blank());
              }}
            >
              {t('另存为新模板', 'Save as a new template')}
            </button>
          </div>
        )}
        {draft ? (
          <form
            id="task-template-form"
            className="task-form"
            onSubmit={(event) => {
              event.preventDefault();
              void save();
            }}
          >
            <label>
              {t('模板名称', 'Template name')}
              <input
                className="ui-input"
                autoFocus
                value={draft.name}
                maxLength={TEMPLATE_NAME_LIMIT}
                disabled={actions.locked}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                required
              />
            </label>
            <label>
              {t('简短说明', 'Description')}
              <input
                className="ui-input"
                value={draft.description}
                maxLength={2000}
                disabled={actions.locked}
                onChange={(e) => setDraft({ ...draft, description: e.target.value })}
              />
            </label>
            <label>
              {t('任务要求', 'Task instructions')}
              <textarea
                className="ui-input"
                rows={8}
                value={draft.instructions}
                maxLength={8000}
                disabled={actions.locked}
                onChange={(e) => setDraft({ ...draft, instructions: e.target.value })}
                required
              />
            </label>
          </form>
        ) : (
          <>
            <div className="task-template-tools">
              <input
                className="ui-input"
                type="search"
                aria-label={t('搜索任务模板', 'Search task templates')}
                placeholder={t('搜索模板', 'Search templates')}
                value={query}
                onChange={(e) => {
                  listScroll.current = 0;
                  setQuery(e.target.value);
                  setPage(1);
                }}
              />
              <button
                className="secondary-button"
                disabled={actions.locked}
                onClick={() => edit(blank())}
              >
                <Plus size={16} />
                {t('新建模板', 'New template')}
              </button>
            </div>
            <p className="muted">
              {t(
                '模板帮助填写任务要求，选用后仍可修改。也可以不使用模板。',
                'Templates help fill in instructions. You can edit them for this task or work without a template.',
              )}
            </p>
            <ul className="task-template-list" aria-busy={loading}>
              {data?.items.map((item) => (
                <li key={item.id}>
                  <div className="task-template-copy">
                    <strong>{item.name}</strong>
                    <p>{item.description}</p>
                    <details>
                      <summary>{t('查看任务要求', 'View instructions')}</summary>
                      <p className="task-preserve-text">{item.instructions}</p>
                    </details>
                  </div>
                  <div className="task-row-actions">
                    <button
                      className="secondary-button"
                      disabled={actions.locked || loading}
                      onClick={() => onUse(item)}
                    >
                      {t('使用', 'Use')}
                    </button>
                    <button
                      className="icon-button"
                      aria-label={`${t('编辑模板', 'Edit template')} ${item.name}`}
                      disabled={actions.locked || loading}
                      onClick={() =>
                        edit({
                          id: item.id,
                          base_row_version: item.row_version,
                          name: item.name,
                          description: item.description,
                          instructions: item.instructions,
                        })
                      }
                    >
                      <Pencil size={16} />
                    </button>
                    <button
                      className="icon-button"
                      aria-label={`${t('复制模板', 'Copy template')} ${item.name}`}
                      disabled={actions.locked || loading}
                      onClick={() =>
                        edit({
                          ...blank(),
                          name: copyName(item.name, t(' 副本', ' copy')),
                          description: item.description,
                          instructions: item.instructions,
                        })
                      }
                    >
                      <Copy size={16} />
                    </button>
                    <button
                      className="icon-button"
                      aria-label={`${t('删除模板', 'Delete template')} ${item.name}`}
                      disabled={actions.locked || loading}
                      onClick={() => setDeleting(item)}
                    >
                      <Trash2 size={16} />
                    </button>
                  </div>
                </li>
              ))}
            </ul>
            {!loading && !data?.items.length && !error && (
              <p>
                {t(
                  '没有匹配的模板，可以直接新建。',
                  'No matching templates. Create one to get started.',
                )}
              </p>
            )}
            {(error || actions.error) && !actions.pending && (
              <button
                onClick={() => {
                  actions.clearError();
                  setReload((n) => n + 1);
                }}
              >
                {t('重新读取', 'Reload')}
              </button>
            )}
          </>
        )}
      </div>
      <footer className="task-template-footer">
        {draft ? (
          <>
            <button
              className="secondary-button"
              disabled={actions.locked}
              onClick={() =>
                leave(() => {
                  actions.clearError();
                  setError('');
                  setDraft(null);
                })
              }
            >
              {t('返回模板', 'Back to templates')}
            </button>
            <button
              className="primary-button"
              form="task-template-form"
              type="submit"
              disabled={actions.locked || !draft.name.trim() || !draft.instructions.trim()}
            >
              {actions.busy ? t('正在保存…', 'Saving…') : t('保存模板', 'Save template')}
            </button>
          </>
        ) : (
          <Pagination
            index={data?.page ?? page}
            size={size}
            count={data?.items.length ?? 0}
            total={data?.total ?? 0}
            loading={loading}
            disabled={actions.locked}
            canPrevious={page > 1}
            canNext={!!data && page * size < data.total}
            onSize={(n) => {
              listScroll.current = 0;
              setSize(n);
              setPage(1);
            }}
            onPrevious={() => {
              listScroll.current = 0;
              setPage((n) => n - 1);
            }}
            onNext={() => {
              listScroll.current = 0;
              setPage((n) => n + 1);
            }}
            onGoTo={(n) => {
              listScroll.current = 0;
              setPage(n);
            }}
          />
        )}
      </footer>
      {discard && (
        <ActionConfirmDialog
          title={t('放弃模板修改？', 'Discard template changes?')}
          description={t(
            '当前尚未保存的修改将被放弃。',
            'Your unsaved template changes will be discarded.',
          )}
          confirmLabel={t('放弃修改', 'Discard changes')}
          onCancel={() => setDiscard(null)}
          onConfirm={() => {
            const next = discard;
            setDiscard(null);
            next();
          }}
        />
      )}
      {deleting && (
        <ActionConfirmDialog
          title={t('删除这个模板？', 'Delete this template?')}
          description={t(
            `删除“${deleting.name}”后，已有 ${deleting.used_by_tasks} 次任务的内容和历史仍会保留。`,
            `Deleting “${deleting.name}” keeps all ${deleting.used_by_tasks} existing tasks and their history.`,
          )}
          confirmLabel={t('删除模板', 'Delete template')}
          tone="danger"
          onCancel={() => setDeleting(null)}
          onConfirm={() => void remove(deleting)}
        />
      )}
    </SourceLibraryDialog>
  );
}

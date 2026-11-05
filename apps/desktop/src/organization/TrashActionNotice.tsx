import { AlertCircle, CheckCircle2, LoaderCircle, RefreshCw, X } from 'lucide-react';
import { useI18n } from '../i18n';
import type { TrashActionSnapshot } from './trash-actions';
import { groupTrashFailures } from './trash-restore-summary';
import './trash-page.css';

export function TrashActionNotice({
  state,
  onRetry,
  onDismiss,
}: {
  state: TrashActionSnapshot;
  onRetry: () => void;
  onDismiss: () => void;
}) {
  const { t } = useI18n();
  const done = state.result?.items.filter((item) => item.status === 'done').length ?? 0;
  // One row per failure repeated the same reason for every item in a large batch,
  // so the list is grouped by reason and the count carries the size.
  const failureGroups = groupTrashFailures(
    (state.result?.items ?? [])
      .filter((item) => item.status === 'failed')
      .map((item) => ({
        object_id: item.object_id,
        title: item.title,
        code: item.code,
        error: item.error,
      })),
  );
  const failedCount = failureGroups.reduce((total, group) => total + group.count, 0);
  const pending = state.pending;
  if (state.busy)
    return (
      <p className="org-working trash-action-working" role="status">
        <LoaderCircle size={15} className="spin" aria-hidden="true" />
        {pending?.kind === 'delete'
          ? t(
              `正在确认彻底删除的 ${pending.preview.total_count} 项…`,
              `Confirming permanent deletion of ${pending.preview.total_count} ${pending.preview.total_count === 1 ? 'item' : 'items'}…`,
            )
          : t(
              `正在恢复…已确认 ${done} / ${pending?.kind === 'restore' ? pending.items.length : 0} 项`,
              `Restoring… ${done} / ${pending?.kind === 'restore' ? pending.items.length : 0} confirmed`,
            )}
      </p>
    );
  if (!pending && !state.error && !state.result) return null;
  const needsAttention = !!pending || !!state.error || failedCount > 0;
  return (
    <div className="org-notice trash-action-notice" role={needsAttention ? 'alert' : 'status'}>
      {needsAttention ? (
        <AlertCircle size={17} aria-hidden="true" />
      ) : (
        <CheckCircle2 size={17} aria-hidden="true" />
      )}
      <div>
        {pending ? (
          <>
            <strong>
              {pending.kind === 'delete' && pending.commit
                ? t('删除已完成，确认记录仍需更新', 'Deletion completed; confirmation needs saving')
                : t('上次回收站操作仍需确认', 'The previous Trash operation needs confirmation')}
            </strong>
            <p>
              {pending.kind === 'delete'
                ? t(
                    `已保留本次确认的 ${pending.preview.total_count} 个目标。重试只核对同一次操作，不会扩大删除范围。确认前暂停新的修改。`,
                    `The ${pending.preview.total_count} confirmed targets are kept. Retrying checks the same operation without expanding its scope. New changes are paused until confirmation.`,
                  )
                : t(
                    `已确认恢复 ${done} 项，仍保留这批原操作。重试将继续核对剩余结果，不会重复恢复已确认项目。`,
                    `${done} restorations are confirmed. The original batch is kept; retry checks the remaining results without repeating confirmed items.`,
                  )}
            </p>
          </>
        ) : state.error ? (
          <>
            <strong>
              {state.code === 'query_expired' || state.code === 'metadata_conflict'
                ? t('内容已有变化，本次删除未执行', 'Items changed; this deletion was not applied')
                : t('回收站操作未完成', 'Trash operation did not complete')}
            </strong>
            {(state.code === 'query_expired' || state.code === 'metadata_conflict') && (
              <p>
                {t(
                  '请重新读取回收站，再检查并确认当前目标。不会自动改用新的删除范围。',
                  'Reload Trash, then review and confirm the current targets. The deletion scope will not change automatically.',
                )}
              </p>
            )}
          </>
        ) : state.result ? (
          <>
            <strong>
              {state.result.kind === 'delete'
                ? t(
                    `已彻底删除 ${done} 项`,
                    `${done} ${done === 1 ? 'item' : 'items'} permanently deleted`,
                  )
                : failedCount
                  ? t(
                      `已恢复 ${done} 项，${failedCount} 项未恢复`,
                      `${done} ${done === 1 ? 'item' : 'items'} restored; ${failedCount} not restored`,
                    )
                  : t(`已恢复 ${done} 项`, `${done} ${done === 1 ? 'item' : 'items'} restored`)}
            </strong>
            {failureGroups.length > 0 && (
              <>
                <p>
                  {t(
                    '未恢复的项目保持原样。重新读取回收站后，可以检查并重新选择。',
                    'Items that were not restored are unchanged. Reload Trash to review and select them again.',
                  )}
                </p>
                <details>
                  <summary>
                    {t(
                      `查看未恢复项目（${failureGroups.length} 类原因）`,
                      `View items not restored (${failureGroups.length} ${failureGroups.length === 1 ? 'reason' : 'reasons'})`,
                    )}
                  </summary>
                  <ul>
                    {failureGroups.map((group) => (
                      <li key={group.key}>
                        <strong title={group.code ?? undefined}>
                          {t(
                            `${group.count} 项 · ${group.message}`,
                            `${group.count} ${group.count === 1 ? 'item' : 'items'} · ${group.message}`,
                          )}
                        </strong>
                        <span>
                          {group.titles
                            .map((title) => title || t('未命名项目', 'Untitled item'))
                            .join(' · ')}
                        </span>
                        {group.remaining > 0 && (
                          <small>
                            {t(`等 ${group.remaining} 项`, `and ${group.remaining} more`)}
                          </small>
                        )}
                      </li>
                    ))}
                  </ul>
                </details>
              </>
            )}
          </>
        ) : null}
        {state.error && <p className="trash-action-error">{state.error}</p>}
        {pending && (
          <button type="button" className="secondary-button" onClick={onRetry}>
            <RefreshCw size={15} aria-hidden="true" />
            {t('重试原操作', 'Retry original operation')}
          </button>
        )}
      </div>
      {!pending && (
        <button
          type="button"
          className="icon-button trash-notice-dismiss"
          onClick={onDismiss}
          aria-label={t('关闭操作结果', 'Dismiss operation result')}
        >
          <X size={16} aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

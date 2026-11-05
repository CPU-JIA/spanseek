import { GitCommitHorizontal, X } from 'lucide-react';
import { useEffect, useId, useRef } from 'react';
import { useI18n } from '../i18n';
import './ai-apply-confirm.css';

export type ProposalApplySummary = {
  added: number;
  changed: number;
  removed: number;
  total: number;
};

export function ProposalApplyConfirmDialog({
  noteTitle,
  baseRevisionId,
  summary,
  busy,
  onCancel,
  onConfirm,
}: {
  noteTitle: string;
  baseRevisionId: string;
  summary: ProposalApplySummary;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const descriptionId = useId();

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    element.showModal();
    cancel.current?.focus({ preventScroll: true });
    return () => {
      if (element.open) element.close();
      if (previous?.isConnected && !previous.matches(':disabled') && !previous.closest('[inert]'))
        previous.focus({ preventScroll: true });
    };
  }, []);

  return (
    <dialog
      ref={dialog}
      role="alertdialog"
      className="ai-apply-confirm-dialog"
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onCancel();
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && (event.nativeEvent.isComposing || event.keyCode === 229)) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
    >
      <header>
        <span className="ai-apply-confirm-icon">
          <GitCommitHorizontal size={22} strokeWidth={1.7} aria-hidden="true" />
        </span>
        <h2 id={titleId}>{t('确认写入这份提案？', 'Apply this proposal?')}</h2>
        <button
          type="button"
          className="icon-button"
          aria-label={t('取消应用', 'Cancel applying')}
          disabled={busy}
          onClick={onCancel}
        >
          <X size={18} aria-hidden="true" />
        </button>
      </header>
      <div className="ai-apply-confirm-content">
        <p id={descriptionId}>
          {t(
            `将把「${noteTitle}」整份写入为新的可恢复修订。不能逐块挑选；应用后可回到版本历史恢复。`,
            `The whole proposal for “${noteTitle}” will be written as a new recoverable revision. Blocks cannot be selected individually, and the previous revision remains available in history.`,
          )}
        </p>
        <p className="ai-apply-confirm-summary">
          {t(
            `新增 ${summary.added} · 修改 ${summary.changed} · 移除 ${summary.removed} · 共 ${summary.total} 个变更块`,
            `${summary.added} added · ${summary.changed} changed · ${summary.removed} removed · ${summary.total} changed blocks`,
          )}
        </p>
        {baseRevisionId && (
          <p className="ai-apply-confirm-base">
            {t('将核对的基准修订：', 'Base revision to verify: ')}
            <code>{baseRevisionId}</code>
          </p>
        )}
        <p className="ai-apply-confirm-warning">
          {t(
            'AI 提案可能有误。确认前请已对照基准正文与建议正文。',
            'AI proposals can be wrong. Make sure you have compared the base and proposed documents before confirming.',
          )}
        </p>
      </div>
      <footer>
        <button
          ref={cancel}
          type="button"
          className="secondary-button"
          disabled={busy}
          onClick={onCancel}
        >
          {t('取消', 'Cancel')}
        </button>
        <button type="button" className="primary-button" disabled={busy} onClick={onConfirm}>
          <GitCommitHorizontal size={16} aria-hidden="true" />
          {busy ? t('正在写入…', 'Applying…') : t('确认写入新修订', 'Write new revision')}
        </button>
      </footer>
    </dialog>
  );
}

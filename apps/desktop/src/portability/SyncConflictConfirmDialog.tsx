import { GitMerge, X } from 'lucide-react';
import { useEffect, useId, useRef } from 'react';
import { useI18n } from '../i18n';
import './sync-conflict-confirm.css';

export type SyncConflictResolution = 'local' | 'remote' | 'new';

export function SyncConflictConfirmDialog({
  mode,
  localTitle,
  remoteTitle,
  localTrashed,
  remoteTombstone,
  busy,
  onCancel,
  onConfirm,
}: {
  mode: SyncConflictResolution;
  localTitle: string;
  remoteTitle: string;
  localTrashed: boolean;
  remoteTombstone: boolean;
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

  const title =
    mode === 'local'
      ? localTrashed
        ? t('保留本机删除？', 'Keep the local deletion?')
        : remoteTombstone
          ? t('保留本机并恢复？', 'Keep local and restore?')
          : t('保留本机版本？', 'Keep the local version?')
      : mode === 'remote'
        ? remoteTombstone
          ? t('采用远端删除？', 'Use the remote deletion?')
          : t('采用远端版本？', 'Use the remote version?')
        : t('另存为新笔记？', 'Save as a new note?');
  const consequence =
    mode === 'local'
      ? localTrashed
        ? t(
            '会让其他设备采用本机的删除状态；对象进入跨设备回收站，而不是继续保留为已编辑正文。',
            'Other devices will adopt this device’s deletion. The object moves to cross-device trash instead of keeping the edited body.',
          )
        : t(
            '会把本机版本作为新的同步版本推送；其他设备下次拉取后采用它。远端当前版本不会被合并进来。',
            'This device’s version is pushed as the new sync version and other devices adopt it on their next pull. The current remote version is not merged in.',
          )
      : mode === 'remote'
        ? remoteTombstone
          ? t(
              '会让本机对象采用远端的删除状态；本机未同步的改动不会自动合并。',
              'This device adopts the remote deletion. Unsynchronized local changes are not merged automatically.',
            )
          : t(
              '会使用远端版本替换本机当前正文；本机未同步的改动不会自动合并。',
              'The remote version replaces this device’s current body. Unsynchronized local changes are not merged automatically.',
            )
        : t(
            '会把本机版本复制成一条新笔记，原对象随后采用远端状态；两边的正文都会保留。',
            'This device’s version is copied to a new note, while the original adopts the remote state. Both bodies remain available.',
          );
  const confirmLabel =
    mode === 'local'
      ? t('确认保留本机', 'Keep local')
      : mode === 'remote'
        ? t('确认采用远端', 'Use remote')
        : t('确认另存新笔记', 'Save as new');

  return (
    <dialog
      ref={dialog}
      role="alertdialog"
      className="sync-conflict-confirm-dialog"
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
        <span className="sync-conflict-confirm-icon">
          <GitMerge size={22} strokeWidth={1.7} aria-hidden="true" />
        </span>
        <h2 id={titleId}>{title}</h2>
        <button
          type="button"
          className="icon-button"
          aria-label={t('取消处理冲突', 'Cancel conflict resolution')}
          disabled={busy}
          onClick={onCancel}
        >
          <X size={18} aria-hidden="true" />
        </button>
      </header>
      <div className="sync-conflict-confirm-content">
        <div className="sync-conflict-confirm-summary">
          <p>
            <strong>{t('本机', 'Local')}</strong>
            <span>
              {localTrashed
                ? t('（本机已删除）', '(deleted locally)')
                : localTitle || t('（无标题）', '(untitled)')}
            </span>
          </p>
          <p>
            <strong>{t('远端', 'Remote')}</strong>
            <span>
              {remoteTombstone
                ? t('（远端已删除）', '(deleted remotely)')
                : remoteTitle || t('（无标题）', '(untitled)')}
            </span>
          </p>
        </div>
        <p id={descriptionId}>{consequence}</p>
        <p className="sync-conflict-confirm-note">
          {t(
            '冲突不会在后台自动解决。确认后仍会重新拉取一次，页面会显示最终结果。',
            'Conflicts are never resolved in the background. After confirmation the app pulls again and shows the final result.',
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
          <GitMerge size={16} aria-hidden="true" />
          {busy ? t('正在处理…', 'Resolving…') : confirmLabel}
        </button>
      </footer>
    </dialog>
  );
}

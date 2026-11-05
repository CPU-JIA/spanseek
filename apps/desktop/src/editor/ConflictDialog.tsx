import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { X } from 'lucide-react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import type { DraftController } from './draft-controller';
import { compareConflict, type ConflictComparison } from './conflict-resolution';
import { ReadDocument } from './ReadDocument';
import './conflict-dialog.css';

export function ConflictDialog({
  controller,
  identity,
  pane,
  onClose,
  onResolved,
  onPreserve,
}: {
  controller: DraftController;
  identity: WorkspaceIdentity;
  pane: string;
  onClose: () => void;
  onResolved: () => void;
  onPreserve: () => Promise<void>;
}) {
  const { t } = useI18n();
  const snapshot = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const dialog = useRef<HTMLDialogElement>(null);
  const cancelDiscard = useRef<HTMLButtonElement>(null);
  const discardButton = useRef<HTMLButtonElement>(null);
  const sequence = useRef(0);
  const live = useRef(true);
  const [comparison, setComparison] = useState<ConflictComparison | null>(null);
  const [loading, setLoading] = useState(false);
  const [acting, setActing] = useState(false);
  const actionLock = useRef(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const stale = comparison !== null && comparison.local !== snapshot.record;
  const blocked = loading || acting || snapshot.historyBusy || !!snapshot.record.historyIntent;
  const loadCurrent = () =>
    call({ kind: 'notes_get', payload: { note_id: snapshot.record.base.id } }, 'note', identity);

  async function reload() {
    const ticket = ++sequence.current;
    setLoading(true);
    setComparison(null);
    setConfirming(false);
    setError(null);
    try {
      const result = await compareConflict(controller, loadCurrent);
      if (live.current && ticket === sequence.current) setComparison(result);
    } catch (cause) {
      if (live.current && ticket === sequence.current) setError(message(cause));
    } finally {
      if (live.current && ticket === sequence.current) setLoading(false);
    }
  }
  useEffect(() => {
    live.current = true;
    dialog.current?.showModal();
    void reload();
    return () => {
      live.current = false;
      sequence.current++;
    };
  }, [controller, identity.local_instance_id, identity.workspace_id]);
  useEffect(() => {
    if (confirming) cancelDiscard.current?.focus();
  }, [confirming]);

  function cancelConfirmation() {
    setConfirming(false);
    requestAnimationFrame(() => discardButton.current?.focus());
  }

  async function resolve(discard: boolean) {
    if (actionLock.current || blocked || !comparison || stale) return;
    actionLock.current = true;
    setActing(true);
    setError(null);
    try {
      if (discard)
        await controller.discardConflict(
          comparison.local,
          comparison.current.revision_id,
          loadCurrent,
        );
      else await onPreserve();
      if (live.current) onResolved();
    } catch (cause) {
      if (live.current) {
        setError(message(cause));
        if (discard) cancelConfirmation();
      }
    } finally {
      actionLock.current = false;
      if (live.current) setActing(false);
    }
  }

  return (
    <dialog
      ref={dialog}
      className="history-dialog conflict-dialog"
      aria-labelledby={`conflict-title-${pane}`}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && (event.nativeEvent.isComposing || event.keyCode === 229)) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
      onCancel={(event) => {
        event.preventDefault();
        if (!actionLock.current) {
          if (confirming) cancelConfirmation();
          else onClose();
        }
      }}
    >
      <header>
        <h2 id={`conflict-title-${pane}`}>{t('保存冲突比较', 'Compare save conflict')}</h2>
        <button
          className="icon-button"
          aria-label={t('关闭比较', 'Close comparison')}
          disabled={acting}
          onClick={onClose}
          autoFocus
        >
          <X size={18} />
        </button>
      </header>
      <p className="muted">
        {t(
          '对照知识库最新版本与本地草稿，再选择如何处理。关闭比较会保留草稿。',
          'Compare the latest saved version and your local draft, then choose how to proceed. Closing keeps the draft.',
        )}
      </p>
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      {stale && (
        <p role="alert" className="inline-error">
          {t(
            '本地草稿已变化，请重新比较后处理。',
            'The local draft changed. Compare again before proceeding.',
          )}
        </p>
      )}
      <button className="secondary-button" disabled={blocked} onClick={() => void reload()}>
        {t('重新比较', 'Compare again')}
      </button>
      {loading && <p role="status">{t('正在读取最新版本…', 'Loading the latest version…')}</p>}
      {comparison && (
        <>
          <div className="conflict-comparison">
            <section aria-label={t('知识库最新版本', 'Latest saved version')}>
              <h3>{t('知识库最新版本', 'Latest saved version')}</h3>
              <p className="muted">
                {t(
                  `第 ${comparison.current.revision} 版`,
                  `Version ${comparison.current.revision}`,
                )}
              </p>
              <h4>{comparison.current.title}</h4>
              {comparison.current.trashed && (
                <p role="alert">{t('此笔记已在回收站。', 'This note is in the trash.')}</p>
              )}
              <ReadDocument note={comparison.current} identity={identity} />
            </section>
            <section aria-label={t('本地草稿', 'Local draft')}>
              <h3>{t('本地草稿', 'Local draft')}</h3>
              <p className="muted">
                {t(
                  `基于第 ${comparison.local.base.revision} 版，尚未提交`,
                  `Based on version ${comparison.local.base.revision}; not committed`,
                )}
              </p>
              <h4>{comparison.local.title}</h4>
              <ReadDocument
                note={{
                  ...comparison.local.base,
                  title: comparison.local.title,
                  document: comparison.local.document,
                }}
                identity={identity}
                imageImports={comparison.local.imageImports}
              />
            </section>
          </div>
          <div className="conflict-actions">
            <button
              className="primary-button"
              disabled={blocked || stale}
              onClick={() => void resolve(false)}
            >
              {t('保留草稿为新笔记', 'Save draft as a new note')}
            </button>
            <button
              ref={discardButton}
              className="secondary-button"
              disabled={blocked || stale || comparison.current.trashed}
              onClick={() => setConfirming(true)}
            >
              {t('放弃草稿并采用知识库版本…', 'Discard draft and use saved version…')}
            </button>
          </div>
          {confirming && (
            <section
              className="conflict-confirmation"
              aria-label={t('确认放弃草稿', 'Confirm discarding draft')}
            >
              <p>
                {t(
                  '将删除这份尚未提交的本地草稿并加载上方知识库版本。此操作不会创建副本，也不会修改知识库正文。',
                  'This removes the uncommitted local draft and loads the saved version shown above. It creates no copy and does not modify saved content.',
                )}
              </p>
              <div className="conflict-actions">
                <button
                  ref={cancelDiscard}
                  className="secondary-button"
                  disabled={acting}
                  onClick={cancelConfirmation}
                >
                  {t('继续保留草稿', 'Keep the draft')}
                </button>
                <button
                  className="primary-button"
                  disabled={blocked || stale}
                  onClick={() => void resolve(true)}
                >
                  {t('确认放弃这份草稿', 'Confirm discarding this draft')}
                </button>
              </div>
            </section>
          )}
          {acting && (
            <p role="status">{t('正在核对并处理…', 'Checking and applying your choice…')}</p>
          )}
        </>
      )}
    </dialog>
  );
}

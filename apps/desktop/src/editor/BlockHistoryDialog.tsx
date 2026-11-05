import { useEffect, useRef, useState } from 'react';
import { History, X } from 'lucide-react';
import type { BlockHistoryPage, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import './block-history.css';

export function BlockHistoryDialog({
  identity,
  noteId,
  blockId,
  onClose,
}: {
  identity: WorkspaceIdentity;
  noteId: string;
  blockId: string;
  onClose: () => void;
}) {
  const { t, language } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const [page, setPage] = useState<BlockHistoryPage | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);

  useEffect(() => {
    dialog.current?.showModal();
    closeButton.current?.focus();
  }, []);

  async function loadFirst() {
    const serial = ++request.current;
    setPage(null);
    setLoading(true);
    setError(null);
    try {
      const result = await call(
        {
          kind: 'block_history',
          payload: { note_id: noteId, block_id: blockId, before: null, limit: 50 },
        },
        'block_history',
        identity,
      );
      if (serial === request.current) setPage(result);
    } catch (cause) {
      if (serial === request.current) setError(message(cause));
    } finally {
      if (serial === request.current) setLoading(false);
    }
  }

  useEffect(() => {
    void loadFirst();
    return () => {
      request.current += 1;
    };
  }, [identity.local_instance_id, identity.workspace_id, noteId, blockId]);

  async function loadOlder() {
    if (!page?.next_before || loadingOlder) return;
    const serial = request.current;
    setLoadingOlder(true);
    setError(null);
    try {
      const older = await call(
        {
          kind: 'block_history',
          payload: {
            note_id: noteId,
            block_id: blockId,
            before: page.next_before,
            limit: 50,
          },
        },
        'block_history',
        identity,
      );
      if (serial !== request.current) return;
      setPage((current) =>
        current
          ? { items: [...current.items, ...older.items], next_before: older.next_before }
          : older,
      );
    } catch (cause) {
      if (serial === request.current) setError(message(cause));
    } finally {
      if (serial === request.current) setLoadingOlder(false);
    }
  }

  return (
    <dialog
      ref={dialog}
      className="block-history-dialog"
      aria-labelledby="block-history-title"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
    >
      <header>
        <div>
          <p className="block-history-eyebrow">
            <History size={15} aria-hidden="true" />
            {t('稳定块身份', 'Stable block identity')}
          </p>
          <h2 id="block-history-title">{t('块历史', 'Block history')}</h2>
        </div>
        <button
          ref={closeButton}
          type="button"
          className="icon-button"
          aria-label={t('关闭块历史', 'Close block history')}
          onClick={onClose}
        >
          <X size={18} aria-hidden="true" />
        </button>
      </header>
      <p className="block-history-help">
        {t(
          '只列出这个块真正发生变化或消失的版本；改标题等无关修订不会混入。',
          'Only revisions where this block changed or disappeared are listed; unrelated title edits are omitted.',
        )}
      </p>
      {error && (
        <div className="inline-error" role="alert">
          <p>{error}</p>
          <button type="button" onClick={() => void loadFirst()}>
            {t('重试', 'Retry')}
          </button>
        </div>
      )}
      {loading && <p role="status">{t('正在读取块历史…', 'Loading block history…')}</p>}
      {!loading && page && !page.items.length && (
        <div className="block-history-empty">
          <History size={26} aria-hidden="true" />
          <p>{t('没有找到可显示的块修订。', 'No block revisions were found.')}</p>
        </div>
      )}
      {!loading && page && page.items.length > 0 && (
        <ol className="block-history-list">
          {page.items.map((item, index) => (
            <li key={item.revision_id} className={item.present ? '' : 'is-missing'}>
              <div className="block-history-meta">
                <strong>
                  {t(`第 ${item.revision} 版`, `Version ${item.revision}`)}
                  {index === 0 && <span>{t('最近变化', 'Latest change')}</span>}
                </strong>
                <time dateTime={item.created_at}>{displayDate(item.created_at, language)}</time>
              </div>
              {item.present ? (
                <>
                  <p className="block-history-text">{item.text || t('空内容块', 'Empty block')}</p>
                  <p className="block-history-detail">
                    {item.kind}
                    {item.alias ? ` · #${item.alias}` : ''}
                    {item.position != null
                      ? ` · ${t(`位置 ${item.position + 1}`, `Position ${item.position + 1}`)}`
                      : ''}
                    {item.depth != null
                      ? ` · ${t(`层级 ${item.depth}`, `Depth ${item.depth}`)}`
                      : ''}
                    {item.parent_block_id
                      ? ` · ${t('父块', 'Parent')} ${item.parent_block_id.slice(-8)}`
                      : ''}
                  </p>
                </>
              ) : (
                <p className="block-history-text">
                  {t(
                    '此版本中不存在该块：它尚未创建或被删除。',
                    'The block is absent in this revision: it was not created yet or was deleted.',
                  )}
                </p>
              )}
            </li>
          ))}
        </ol>
      )}
      {!loading && page?.next_before && (
        <button
          type="button"
          className="secondary-button block-history-more"
          disabled={loadingOlder}
          onClick={() => void loadOlder()}
        >
          {loadingOlder ? t('正在读取…', 'Loading…') : t('更早的块修订', 'Earlier block revisions')}
        </button>
      )}
      <footer>
        <button type="button" className="primary-button" onClick={onClose}>
          {t('关闭', 'Close')}
        </button>
      </footer>
    </dialog>
  );
}

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { SourceCollectionPicker, type SourceCollectionChoice } from './SourceCollectionPicker';
import {
  prepareSourceBulk,
  SourceBulkActions,
  sourceBulkKey,
  type SelectedSource,
  type SourceBulkAction,
} from './source-bulk-actions';
import { sourceBulkStorage } from './source-bulk-storage';
import { sourceReading } from './reading-registry';
import './source-bulk.css';

const controllers = new Map<string, SourceBulkActions>();
function getController(identity: WorkspaceIdentity) {
  const key = sourceBulkKey(identity);
  let controller = controllers.get(key);
  if (!controller) {
    controller = new SourceBulkActions(
      identity,
      sourceBulkStorage,
      async (selected, action) => {
        if (action.kind !== 'trash' || action.value)
          await Promise.all(selected.map((item) => sourceReading(identity, item.id).flush()));
        return prepareSourceBulk(
          selected,
          action,
          (id) =>
            call(
              { kind: 'objects_metadata_get', payload: { object_id: id } },
              'object_metadata',
              identity,
            ),
          (id) =>
            call(
              { kind: 'collections_get', payload: { collection_id: id } },
              'collection',
              identity,
            ),
        );
      },
      (step) => call(step.command, step.expected, identity),
    );
    controllers.set(key, controller);
  }
  return controller;
}

export function SourceBulkOrganization({
  identity,
  selected,
  trashed,
  onClear,
  onChanged,
}: {
  identity: WorkspaceIdentity;
  selected: SelectedSource[];
  trashed: boolean;
  onClear: () => void;
  onChanged: () => void;
}) {
  const { t } = useI18n();
  const controller = useMemo(
    () => getController(identity),
    [identity.local_instance_id, identity.workspace_id],
  );
  const { ready, busy, record, error, generation } = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const [tagName, setTagName] = useState('');
  const [collection, setCollection] = useState<SourceCollectionChoice | null>(null);
  const [expanded, setExpanded] = useState(false);
  const previous = useRef(generation);
  useEffect(() => {
    void controller.load();
  }, [controller]);
  useEffect(() => {
    if (!busy && generation !== previous.current) {
      previous.current = generation;
      onChanged();
    }
  }, [busy, generation, onChanged]);
  useEffect(() => {
    if (record || error) setExpanded(true);
    else if (!selected.length) setExpanded(false);
  }, [!!record, error, selected.length]);
  const locked = !ready || busy || !!record;
  if (!selected.length && !record && !error) return null;
  const completed =
    record?.steps.filter((step) => step.status === 'done').flatMap((step) => step.ids).length ?? 0;
  const failed =
    record?.steps.filter((step) => step.status === 'failed').flatMap((step) => step.ids).length ??
    0;
  const unresolved = record?.steps.some(
    (step) => step.status === 'pending' || step.status === 'ready',
  );
  function actionName(action: SourceBulkAction) {
    if (action.kind === 'tag') return t(`添加标签「${action.name}」`, `Add tag “${action.name}”`);
    if (action.kind === 'collection')
      return t(`加入集合「${action.name}」`, `Add to “${action.name}”`);
    if (action.kind === 'trash')
      return action.value ? t('移入回收站', 'Move to trash') : t('恢复资料', 'Restore documents');
    return action.value ? t('启用检索', 'Enable search') : t('停用检索', 'Disable search');
  }
  return (
    <details
      className="source-bulk-panel"
      aria-label={t('批量整理资料', 'Organize selected documents')}
      aria-busy={busy}
      open={expanded}
      onToggle={(event) => setExpanded(event.currentTarget.open)}
    >
      <summary>
        <strong>
          {record
            ? actionName(record.action)
            : t(`整理 ${selected.length} 份资料`, `Organize ${selected.length} documents`)}
        </strong>
        <span>{expanded ? t('收起操作', 'Collapse actions') : t('展开操作', 'Show actions')}</span>
      </summary>
      <div className="source-bulk-body">
        <div className="source-tools">
          <strong>
            {record
              ? actionName(record.action)
              : t(`已选 ${selected.length} 份资料`, `${selected.length} documents selected`)}
          </strong>
          {!record && (
            <button type="button" disabled={busy} onClick={onClear}>
              {t('清除选择', 'Clear selection')}
            </button>
          )}
        </div>
        {!record && (
          <>
            {!trashed && (
              <>
                <form
                  className="source-tools"
                  onSubmit={(event) => {
                    event.preventDefault();
                    if (tagName.trim())
                      void controller.start(selected, { kind: 'tag', name: tagName.trim() });
                  }}
                >
                  <label>
                    {t('为所选资料添加标签', 'Tag selected documents')}
                    <input
                      disabled={locked}
                      value={tagName}
                      maxLength={100}
                      onChange={(event) => setTagName(event.target.value)}
                    />
                  </label>
                  <button disabled={locked || !tagName.trim()}>{t('添加标签', 'Add tag')}</button>
                </form>
                <div className="source-tools">
                  <SourceCollectionPicker
                    identity={identity}
                    value={collection}
                    onChange={setCollection}
                    disabled={locked}
                    label={t('加入集合', 'Add to collection')}
                  />
                  <button
                    type="button"
                    disabled={locked || !collection}
                    onClick={() => {
                      if (collection)
                        void controller.start(selected, { kind: 'collection', ...collection });
                    }}
                  >
                    {t('加入所选集合', 'Add to selected collection')}
                  </button>
                </div>
              </>
            )}
            <div className="source-tools">
              {!trashed && (
                <>
                  <button
                    type="button"
                    disabled={locked}
                    onClick={() =>
                      void controller.start(selected, { kind: 'search', value: false })
                    }
                  >
                    {t('停用检索', 'Disable search')}
                  </button>
                  <button
                    type="button"
                    disabled={locked}
                    onClick={() => void controller.start(selected, { kind: 'search', value: true })}
                  >
                    {t('启用检索', 'Enable search')}
                  </button>
                </>
              )}
              <button
                type="button"
                disabled={locked}
                onClick={() => void controller.start(selected, { kind: 'trash', value: !trashed })}
              >
                {trashed
                  ? t(`恢复 ${selected.length} 份资料`, `Restore ${selected.length} documents`)
                  : t(`回收 ${selected.length} 份资料`, `Trash ${selected.length} documents`)}
              </button>
            </div>
            <p>
              {t(
                '为所选资料添加标签、加入集合或调整检索。回收后的资料仍可恢复。',
                'Tag documents, add them to a collection or change search availability. Trashed documents can be restored.',
              )}
            </p>
          </>
        )}
        {record && (
          <>
            <p role="status">
              {t(
                `${completed} / ${record.selected.length} 份已完成，${failed} 份未完成`,
                `${completed} / ${record.selected.length} completed, ${failed} failed`,
              )}
              {busy ? t(' · 正在处理…', ' · Working…') : ''}
            </p>
            <details open={failed > 0 || record.steps.some((step) => step.status === 'pending')}>
              <summary>{t('逐项结果', 'Per-document results')}</summary>
              <ul className="source-bulk-results">
                {record.selected.map((item) => {
                  const step = record.steps.find((value) => value.ids.includes(item.id))!;
                  return (
                    <li key={item.id}>
                      <strong>{item.title}</strong>
                      <span>
                        {step.status === 'done'
                          ? t('已完成', 'Completed')
                          : step.status === 'failed'
                            ? t('未完成', 'Failed')
                            : step.status === 'pending'
                              ? t('待确认', 'Unconfirmed')
                              : t('待处理', 'Waiting')}
                      </span>
                      {step.error && <small>{step.error}</small>}
                    </li>
                  );
                })}
              </ul>
            </details>
            {unresolved ? (
              <>
                <p>
                  {t(
                    '未确认的请求保留在本机；确认后继续余项。已完成项不会重做。',
                    'Unconfirmed requests are saved locally. Confirm to continue remaining items; completed items are skipped.',
                  )}
                </p>
                <button type="button" disabled={busy} onClick={() => void controller.resume()}>
                  {t('继续并确认结果', 'Continue and confirm results')}
                </button>
              </>
            ) : (
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  void controller.dismiss().then((done) => {
                    if (done) onClear();
                  })
                }
              >
                {t('完成', 'Done')}
              </button>
            )}
          </>
        )}
        {error && <p role="alert">{error}</p>}
        {!ready && (
          <button type="button" onClick={() => void controller.load()}>
            {t('重新读取整理记录', 'Reload organization records')}
          </button>
        )}
      </div>
    </details>
  );
}

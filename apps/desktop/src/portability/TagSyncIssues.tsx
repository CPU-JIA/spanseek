import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import {
  AlertCircle,
  ArrowLeft,
  ChevronRight,
  ExternalLink,
  Hash,
  RefreshCw,
  Tags,
} from 'lucide-react';
import type {
  ObjectKind,
  TagDetails,
  TagSyncLabel,
  TagSyncPage,
  TagSyncPayload,
  TagSyncPendingReason,
  TagSyncSnapshot,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { ListPagination } from '../layout/Pagination.tsx';
import { usePageNavigationPaging } from '../layout/usePageNavigationPaging';
import { PageState } from '../layout/PageState';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import './tag-sync-issues.css';

type Translate = (zh: string, en: string) => string;
export type TagSyncResolution = 'local' | 'remote';

export type TagSyncIssuesProps = {
  identity: WorkspaceIdentity;
  refreshVersion: number;
  /** null means the connection state has not been read, rather than unconfigured. */
  configured: boolean | null;
  canPush: boolean;
  canReadRemote: boolean;
  /** The adapter must query pending_only=true and return the matching exact total. */
  readPage: (after: string | null, limit: number) => Promise<TagSyncPage>;
  onRetry: (snapshot: TagSyncSnapshot) => Promise<TagSyncSnapshot>;
  onResolve: (snapshot: TagSyncSnapshot, mode: TagSyncResolution) => Promise<TagSyncSnapshot>;
  onOpenItem: (snapshot: TagSyncSnapshot) => void;
  onOpenTags: () => void;
  onOpenView?: (viewId: string) => void;
  onChanged: () => void | Promise<void>;
};

const identityKey = (identity: WorkspaceIdentity) =>
  `${identity.local_instance_id}/${identity.workspace_id}`;

export function tagSyncReason(reason: TagSyncPendingReason, t: Translate) {
  switch (reason) {
    case 'local_changes':
      return {
        title: t('两端修改不同', 'Both devices have changes'),
        detail: t(
          '本机与远端的修改都已保留。比较后选择保留哪一边，不会自动合并。',
          'Local and remote changes are both retained. Compare them and choose one; they are not merged automatically.',
        ),
      };
    case 'name_conflict':
      return {
        title: t('与本机同名标签冲突', 'A different local tag has this name'),
        detail: t(
          '同名标签有不同身份。先在标签管理中核对并改名，再重新检查；不会按名称合并标签。',
          'These names belong to different tag identities. Review and rename the local tag, then check again. Tags are not merged by name.',
        ),
      };
    case 'missing_object':
      return {
        title: t('等待对应内容', 'Waiting for the content'),
        detail: t(
          '本机还没有对应对象。已保留这份标签分配，待同一对象到达后再检查。',
          'The matching object is not on this device. Its tag assignments are retained until that same object becomes available.',
        ),
      };
    case 'wrong_object_kind':
      return {
        title: t('对象类型不一致', 'Object types do not match'),
        detail: t(
          '本机对象与远端分配要求的类型不同。请先核对知识库和内容，本面板不会更换对象类型。',
          'The local object has a different type from the remote assignment. Check the workspace and content first; this panel will not change its type.',
        ),
      };
    case 'missing_tags':
      return {
        title: t('等待标签定义', 'Waiting for tag definitions'),
        detail: t(
          '部分标签定义尚未到达。原标签身份和分配已保留，定义到达后可重新检查。',
          'Some tag definitions have not arrived. Their identities and assignments are retained and can be checked again when the definitions arrive.',
        ),
      };
    case 'deleted_tags':
      return {
        title: t('引用了已删除的标签', 'The assignments reference deleted tags'),
        detail: t(
          '远端分配仍引用已删除的标签。请核对标签管理和来源设备；不会悄悄重建标签或丢弃这些分配。',
          'The remote assignments still reference deleted tags. Review tag management and the source device; these tags are not silently recreated or dropped.',
        ),
      };
    case 'view_dependency':
      return {
        title: t('保存视图仍在使用', 'Saved views still use this tag'),
        detail: t(
          '本机保存视图仍引用这个标签。先调整列出的视图筛选，再重新检查删除。',
          'Local saved views still reference this tag. Update the listed view filters, then check the deletion again.',
        ),
      };
    case 'assignment_changes':
      return {
        title: t('标签分配有本机修改', 'Local tag assignments have changed'),
        detail: t(
          '远端要删除的标签在本机仍有未确认的分配修改。请核对删除影响，再决定保留本机或采用删除。',
          'The tag being deleted remotely has unconfirmed local assignment changes. Review the impact before keeping local state or accepting the deletion.',
        ),
      };
    case 'deleted_tag':
      return {
        title: t('本机标签已删除', 'The local tag was deleted'),
        detail: t(
          '本机保留了删除记录，远端仍保留标签定义。请明确选择要保留的状态。',
          'This device retains a deletion record while the remote device retains a tag definition. Choose explicitly which state to keep.',
        ),
      };
  }
}

function objectKindLabel(kind: ObjectKind | undefined, t: Translate) {
  switch (kind) {
    case 'note':
      return t('笔记', 'Note');
    case 'source':
      return t('资料', 'Source');
    case 'annotation':
      return t('摘录', 'Annotation');
    case 'collection':
      return t('集合', 'Collection');
    default:
      return t('内容', 'Content');
  }
}

function assignmentKind(snapshot: TagSyncSnapshot) {
  const payload = snapshot.payload ?? snapshot.pending?.payload;
  return payload?.sync_kind === 'tag_assignments' ? payload.assignments.object_kind : undefined;
}

export function tagSyncTitle(snapshot: TagSyncSnapshot, t: Translate) {
  if (snapshot.entity_title?.trim()) return snapshot.entity_title;
  const payload = snapshot.payload ?? snapshot.pending?.payload;
  if (payload?.sync_kind === 'tag') return payload.tag.display_name;
  const kind = objectKindLabel(assignmentKind(snapshot), t);
  return snapshot.local_exists
    ? t(`未命名${kind}`, `Untitled ${kind.toLowerCase()}`)
    : t(`尚未到达的${kind}`, `${kind} not yet available`);
}

function isDeletion(payload: TagSyncPayload | null | undefined) {
  return payload?.sync_kind === 'tag' && payload.tag.deleted_at !== null;
}

/** Copy all CAS fields and material at selection time, including nested impact details. */
export function freezeTagSyncDecision(snapshot: TagSyncSnapshot): TagSyncSnapshot {
  const copy = structuredClone(snapshot);
  const freeze = (value: unknown) => {
    if (value && typeof value === 'object') {
      for (const child of Object.values(value)) freeze(child);
      Object.freeze(value);
    }
  };
  freeze(copy);
  return copy;
}

export function tagSyncAvailability(
  snapshot: TagSyncSnapshot,
  permissions: Pick<TagSyncIssuesProps, 'configured' | 'canPush' | 'canReadRemote'>,
) {
  const readable = permissions.configured === true && permissions.canReadRemote;
  const reason = snapshot.pending?.reason;
  const hasLocal =
    snapshot.payload !== null &&
    snapshot.local_fingerprint !== null &&
    (snapshot.kind === 'tag' || snapshot.local_exists);
  const blockedRemote =
    reason !== undefined &&
    [
      'name_conflict',
      'missing_object',
      'wrong_object_kind',
      'missing_tags',
      'deleted_tags',
      'view_dependency',
    ].includes(reason);
  const hasDeleteReview =
    !isDeletion(snapshot.pending?.payload) ||
    !snapshot.local_exists ||
    Boolean(snapshot.delete_impact && snapshot.local_impact_token);
  return {
    retry: Boolean(snapshot.pending) && readable,
    local:
      Boolean(snapshot.pending) &&
      readable &&
      permissions.canPush &&
      hasLocal &&
      reason !== 'wrong_object_kind',
    remote: Boolean(snapshot.pending) && readable && !blockedRemote && hasDeleteReview,
    openItem: snapshot.kind === 'tag_assignments' && snapshot.local_exists,
  };
}

export function tagSyncOutcome(snapshot: TagSyncSnapshot, t: Translate) {
  if (snapshot.pending)
    return {
      tone: 'attention' as const,
      text: t(
        `仍需处理：${tagSyncReason(snapshot.pending.reason, t).title}。比较材料已更新。`,
        `Still needs attention: ${tagSyncReason(snapshot.pending.reason, t).title}. The comparison has been updated.`,
      ),
    };
  if (snapshot.dirty)
    return {
      tone: 'attention' as const,
      text: t(
        '这条记录已无待处理决定，本机版本仍待同步确认。',
        'This record no longer needs a decision. Its local version still awaits sync confirmation.',
      ),
    };
  return {
    tone: 'success' as const,
    text: t('这条记录已无待处理决定。', 'This record no longer needs a decision.'),
  };
}

function labelText(tag: TagSyncLabel, t: Translate) {
  const name = tag.display_name ?? t('尚未收到名称的标签', 'Tag name not yet received');
  return tag.deleted ? t(`${name}（已删除）`, `${name} (deleted)`) : name;
}

function impactText(impact: TagDetails, t: Translate) {
  return t(
    `影响 ${impact.object_count} 个对象：${impact.note_count} 篇笔记、${impact.source_count} 份资料、${impact.annotation_count} 条摘录、${impact.collection_count} 个集合，以及 ${impact.saved_views.length} 个保存视图。`,
    `Affects ${impact.object_count} objects: ${impact.note_count} notes, ${impact.source_count} sources, ${impact.annotation_count} annotations, ${impact.collection_count} collections, and ${impact.saved_views.length} saved views.`,
  );
}

export function tagSyncConfirmation(
  snapshot: TagSyncSnapshot,
  mode: TagSyncResolution,
  t: Translate,
) {
  const chosen = mode === 'local' ? snapshot.payload : snapshot.pending?.payload;
  const title = tagSyncTitle(snapshot, t);
  let detail: string;
  if (snapshot.kind === 'tag_assignments') {
    const labels = mode === 'local' ? snapshot.local_tags : snapshot.remote_tags;
    const names = labels.length
      ? labels.map((tag) => labelText(tag, t)).join('、')
      : t('无标签（清空分配）', 'No tags (clear the assignments)');
    detail =
      mode === 'local'
        ? t(
            `将保留「${title}」的本机标签：${names}。这些标签将作为本机版本提交，不与已核对的远端分配合并。`,
            `Keep the local tags for “${title}”: ${names}. Submit this local version without merging the reviewed remote assignments.`,
          )
        : t(
            `将「${title}」的本机标签替换为远端分配：${names}。本机未同步的分配不会自动合并。`,
            `Replace the local tags for “${title}” with the remote assignments: ${names}. Unsynchronized local assignments are not merged automatically.`,
          );
  } else if (isDeletion(chosen)) {
    detail =
      mode === 'local'
        ? t(
            `保留「${title}」的本机删除状态，并将其作为本机版本提交。其他设备的具体影响取决于各自当前数据。`,
            `Keep the local deletion of “${title}” and submit it as the local version. The effect on other devices depends on their current data.`,
          )
        : t(
            `采用远端对「${title}」的删除，移除本机标签定义及其分配。`,
            `Accept the remote deletion of “${title}”, removing its local definition and assignments.`,
          );
    if (mode === 'remote' && snapshot.delete_impact)
      detail += ` ${impactText(snapshot.delete_impact, t)}`;
  } else {
    const tag = chosen?.sync_kind === 'tag' ? chosen.tag : null;
    const definition = tag
      ? t(
          `名称「${tag.display_name}」，说明：${tag.description || '未填写说明'}。`,
          `Name “${tag.display_name}”, description: ${tag.description || 'No description provided'}.`,
        )
      : '';
    detail =
      mode === 'local'
        ? t(
            `保留「${title}」的本机定义，并将其作为本机版本提交。${definition}`,
            `Keep the local definition of “${title}” and submit it as the local version. ${definition}`,
          )
        : t(
            `用已核对的远端定义替换「${title}」的本机状态。${definition}`,
            `Replace the local state of “${title}” with the reviewed remote definition. ${definition}`,
          );
  }
  return `${detail} ${t('正文和原件保留。此面板不提供撤销，请核对后确认。', 'Bodies and original files are retained. This panel has no undo action; review before confirming.')}`;
}

function TagLabels({ tags }: { tags: TagSyncLabel[] }) {
  const { t } = useI18n();
  return tags.length ? (
    <ul className="tag-sync-labels">
      {tags.map((tag) => (
        <li
          key={tag.id}
          className={
            tag.deleted ? 'is-deleted' : tag.display_name === null ? 'is-unknown' : undefined
          }
        >
          {labelText(tag, t)}
        </li>
      ))}
    </ul>
  ) : (
    <p className="tag-sync-muted">{t('无标签', 'No tags')}</p>
  );
}

/** Pure comparison surface; names and missing/deleted states come from the Core snapshot. */
export function TagSyncComparison({
  snapshot,
  onOpenView,
}: {
  snapshot: TagSyncSnapshot;
  onOpenView?: TagSyncIssuesProps['onOpenView'];
}) {
  const { t, language } = useI18n();
  const sides = [
    {
      name: t('本机', 'This device'),
      payload: snapshot.payload,
      tags: snapshot.local_tags,
      local: true,
    },
    {
      name: t('已收到的远端版本', 'Received remote version'),
      payload: snapshot.pending?.payload,
      tags: snapshot.remote_tags,
      local: false,
    },
  ];
  return (
    <>
      <div className="tag-sync-comparison">
        {sides.map((side) => (
          <section key={side.name} aria-label={side.name}>
            <h4>{side.name}</h4>
            {!side.payload ||
            (side.local && !snapshot.local_exists && snapshot.kind === 'tag_assignments') ? (
              <p className="tag-sync-muted">
                {t(
                  side.local
                    ? '本机还没有对应记录，不能把它当作空白版本推送。'
                    : '尚未收到可比较的远端记录。',
                  side.local
                    ? 'No matching local record is available; it cannot be pushed as a blank version.'
                    : 'No remote record has been received for comparison.',
                )}
              </p>
            ) : side.payload.sync_kind === 'tag' ? (
              <>
                <strong className="tag-sync-definition-name">
                  {side.payload.tag.display_name}
                </strong>
                <p className={side.payload.tag.deleted_at ? 'tag-sync-deleted' : 'tag-sync-muted'}>
                  {side.payload.tag.deleted_at
                    ? t('已删除', 'Deleted')
                    : t('保留标签定义', 'Tag definition retained')}
                </p>
                <p className="tag-sync-description">
                  {side.payload.tag.description || t('未填写说明', 'No description provided')}
                </p>
              </>
            ) : (
              <>
                <p className="tag-sync-muted">
                  {objectKindLabel(side.payload.assignments.object_kind, t)}
                </p>
                <TagLabels tags={side.tags} />
              </>
            )}
          </section>
        ))}
      </div>
      {isDeletion(snapshot.pending?.payload) && snapshot.local_exists && (
        <div className="tag-sync-delete-impact">
          <strong>{t('采用远端删除的影响', 'Impact of accepting the remote deletion')}</strong>
          {snapshot.delete_impact ? (
            <>
              <p>{impactText(snapshot.delete_impact, t)}</p>
              {snapshot.delete_impact.trashed_object_count > 0 && (
                <p>
                  {t(
                    `其中 ${snapshot.delete_impact.trashed_object_count} 个对象在回收站中。`,
                    `${snapshot.delete_impact.trashed_object_count} of these objects are in trash.`,
                  )}
                </p>
              )}
              {snapshot.delete_impact.saved_views.length > 0 && (
                <ul className="tag-sync-view-dependencies">
                  {snapshot.delete_impact.saved_views.map((view) => (
                    <li key={view.id}>
                      {onOpenView ? (
                        <button
                          type="button"
                          className="text-button"
                          onClick={() => onOpenView(view.id)}
                        >
                          <ExternalLink size={14} aria-hidden="true" />
                          {view.name}
                        </button>
                      ) : (
                        view.name
                      )}
                    </li>
                  ))}
                </ul>
              )}
              <p>
                {t(
                  '只移除标签定义和分配，正文及原件保留。此面板不能撤销删除。',
                  'Only the tag definition and assignments are removed. Bodies and original files remain. This panel cannot undo deletion.',
                )}
              </p>
            </>
          ) : (
            <p>
              {t(
                '影响清单尚未取得，请刷新后再决定删除。',
                'The impact list is not available yet. Refresh before deciding to delete.',
              )}
            </p>
          )}
        </div>
      )}
      <details className="tag-sync-technical">
        <summary>{t('查看标识与版本', 'Identifiers and versions')}</summary>
        <dl>
          <div>
            <dt>{t('对象标识', 'Entity ID')}</dt>
            <dd>{snapshot.entity_id}</dd>
          </div>
          <div>
            <dt>{t('同步标识', 'Sync ID')}</dt>
            <dd>{snapshot.sync_object_id}</dd>
          </div>
          <div>
            <dt>{t('远端版本', 'Remote version')}</dt>
            <dd>{snapshot.remote_version}</dd>
          </div>
          <div>
            <dt>{t('已确认基准', 'Confirmed base')}</dt>
            <dd>{snapshot.base_version}</dd>
          </div>
          <div>
            <dt>{t('本机代次', 'Local generation')}</dt>
            <dd>{snapshot.local_generation}</dd>
          </div>
          {snapshot.pending && (
            <div>
              <dt>{t('接收时间', 'Received at')}</dt>
              <dd>{displayDate(snapshot.pending.received_at, language)}</dd>
            </div>
          )}
          <div>
            <dt>{t('本机比较标识', 'Local comparison token')}</dt>
            <dd>{snapshot.local_fingerprint ?? t('无本机版本', 'No local version')}</dd>
          </div>
          {snapshot.local_impact_token && (
            <div>
              <dt>{t('删除影响标识', 'Deletion impact token')}</dt>
              <dd>{snapshot.local_impact_token}</dd>
            </div>
          )}
        </dl>
        {(snapshot.local_tags.length > 0 || snapshot.remote_tags.length > 0) && (
          <ul>
            {[
              ...new Map(
                [...snapshot.local_tags, ...snapshot.remote_tags].map((tag) => [tag.id, tag]),
              ).values(),
            ].map((tag) => (
              <li key={tag.id}>
                {labelText(tag, t)}
                <code>{tag.id}</code>
              </li>
            ))}
          </ul>
        )}
      </details>
    </>
  );
}

type Feedback = { tone: 'success' | 'attention' | 'error'; text: string };
type Decision = { snapshot: TagSyncSnapshot; mode: TagSyncResolution };
type FailedAction = { snapshot: TagSyncSnapshot; mode: 'retry' | TagSyncResolution };

export function TagSyncIssues(props: TagSyncIssuesProps) {
  return <TagSyncIssuesScope key={identityKey(props.identity)} {...props} />;
}

function TagSyncIssuesScope(props: TagSyncIssuesProps) {
  const { t } = useI18n();
  const headingId = useId();
  const key = identityKey(props.identity);
  const [selectedId, setSelectedId] = usePageNavigationState<string | null>(
    'sync.tags.selected',
    null,
  );
  const [detailScroll, setDetailScroll] = usePageNavigationState('sync.tags.detail-scroll', 0);
  const [inspection, setInspection] = useState<TagSyncSnapshot | null>(null);
  const [confirmation, setConfirmation] = useState<Decision | null>(null);
  const [failedAction, setFailedAction] = useState<FailedAction | null>(null);
  const [holdComparison, setHoldComparison] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [busy, setBusy] = useState(false);
  const [reload, setReload] = useState(0);
  const detailElement = useRef<HTMLDivElement>(null);
  const feedbackElement = useRef<HTMLDivElement>(null);
  const headingElement = useRef<HTMLHeadingElement>(null);
  const detailHeading = useRef<HTMLHeadingElement>(null);
  const busyRef = useRef(false);
  const life = useRef(true);
  const scope = `${key}/${props.refreshVersion}/${reload}`;
  const paging = usePageNavigationPaging<TagSyncSnapshot>(
    scope,
    'sync.tags',
    async (after, limit) => {
      const result = await props.readPage(after, limit);
      if (
        !Number.isSafeInteger(result.total) ||
        result.total < result.items.length ||
        result.items.some((item) => !item.pending) ||
        new Set(result.items.map((item) => item.sync_object_id)).size !== result.items.length
      )
        throw new Error(
          t(
            '待处理列表返回了不一致的数据，请重新读取。',
            'The pending list returned inconsistent data. Reload it.',
          ),
        );
      return result;
    },
    true,
    !selectedId,
  );
  const list = paging.pager;
  const listElement = paging.element;
  const restoring = !paging.ready && !list.error && !!list.page;
  useEffect(() => {
    life.current = true;
    return () => {
      life.current = false;
    };
  }, []);
  useLayoutEffect(() => {
    if (inspection && detailElement.current) detailElement.current.scrollTop = detailScroll;
  }, [inspection?.sync_object_id]);
  useEffect(() => {
    if (
      !selectedId ||
      list.loading ||
      restoring ||
      !list.page ||
      list.error ||
      busy ||
      confirmation ||
      holdComparison
    )
      return;
    const current = list.page.items.find((item) => item.sync_object_id === selectedId);
    if (current) setInspection(current);
    else {
      setSelectedId(null);
      setInspection(null);
    }
  }, [
    selectedId,
    list.page,
    list.loading,
    list.error,
    restoring,
    busy,
    confirmation,
    holdComparison,
  ]);
  useEffect(() => {
    if (feedback?.tone === 'error') {
      feedbackElement.current?.focus({ preventScroll: true });
      feedbackElement.current?.scrollIntoView({ block: 'nearest' });
    }
  }, [feedback]);

  const loading = list.loading || restoring;
  const pager = list;
  const readOnly = props.canReadRemote && !props.canPush;
  const selected = selectedId && inspection?.sync_object_id === selectedId ? inspection : null;
  const availability = selected ? tagSyncAvailability(selected, props) : null;
  const blocked = busy || loading || Boolean(list.error);
  function open(snapshot: TagSyncSnapshot) {
    paging.rememberScroll();
    setDetailScroll(0);
    setSelectedId(snapshot.sync_object_id);
    setInspection(snapshot);
    setFeedback(null);
    setFailedAction(null);
    setHoldComparison(false);
    requestAnimationFrame(() => detailHeading.current?.focus({ preventScroll: true }));
  }
  function refresh() {
    if (!busyRef.current) {
      setFeedback(null);
      setFailedAction(null);
      setHoldComparison(false);
      setReload((value) => value + 1);
    }
  }
  function decide(snapshot: TagSyncSnapshot, mode: TagSyncResolution) {
    if (busyRef.current || !tagSyncAvailability(snapshot, props)[mode]) return;
    setConfirmation({ snapshot: freezeTagSyncDecision(snapshot), mode });
  }
  async function act(snapshot: TagSyncSnapshot, mode: 'retry' | TagSyncResolution) {
    if (busyRef.current) return;
    if (!tagSyncAvailability(snapshot, props)[mode]) {
      setFeedback({
        tone: 'error',
        text: t(
          '连接、权限或记录状态已变化，尚未执行。请重新读取后再决定。',
          'The connection, permission, or record state changed. Nothing was executed. Reload before deciding again.',
        ),
      });
      return;
    }
    busyRef.current = true;
    setBusy(true);
    setFeedback(null);
    setFailedAction(null);
    try {
      const result =
        mode === 'retry' ? await props.onRetry(snapshot) : await props.onResolve(snapshot, mode);
      if (!life.current) return;
      if (
        result.sync_object_id !== snapshot.sync_object_id ||
        result.entity_id !== snapshot.entity_id ||
        result.kind !== snapshot.kind
      )
        throw new Error(
          t(
            '返回的记录与本次操作不匹配，已保留原比较。请重新读取后核对。',
            'The returned record does not match this operation. The original comparison is retained; reload and check it.',
          ),
        );
      setFeedback(tagSyncOutcome(result, t));
      setHoldComparison(false);
      if (result.pending) setInspection(result);
      else {
        setSelectedId(null);
        setInspection(null);
        requestAnimationFrame(() => headingElement.current?.focus({ preventScroll: true }));
      }
      setReload((value) => value + 1);
      try {
        await props.onChanged();
      } catch (failure) {
        if (life.current)
          setFeedback({
            tone: 'error',
            text: t(
              `本条结果已返回，但关联页面刷新失败：${message(failure)}。请重新读取。`,
              `This record returned a result, but related pages could not refresh: ${message(failure)}. Reload them.`,
            ),
          });
      }
    } catch (failure) {
      if (life.current) {
        setInspection(snapshot);
        setSelectedId(snapshot.sync_object_id);
        setHoldComparison(true);
        setFailedAction({ snapshot, mode });
        setFeedback({
          tone: 'error',
          text: t(
            `本次处理未确认，比较材料仍保留。${message(failure)}`,
            `This action is not confirmed. The comparison is retained. ${message(failure)}`,
          ),
        });
      }
    } finally {
      busyRef.current = false;
      if (life.current) setBusy(false);
    }
  }
  return (
    <section className="tag-sync-issues" aria-labelledby={headingId}>
      <header className="tag-sync-heading">
        <div>
          <h3 id={headingId} ref={headingElement} tabIndex={-1}>
            {t('标签同步待处理', 'Tag sync needs attention')}
            {list.page && <span>{list.page.total}</span>}
          </h3>
          <p>
            {t(
              '先核对差异或缺少的依赖，再决定如何继续。',
              'Review differences or missing dependencies before continuing.',
            )}
          </p>
        </div>
        <button
          type="button"
          className="icon-button"
          aria-label={t('刷新标签待处理记录', 'Refresh pending tag records')}
          disabled={busy || loading}
          onClick={refresh}
        >
          <RefreshCw size={16} aria-hidden="true" />
        </button>
      </header>
      {props.configured === false ? (
        <p className="tag-sync-connection" role="status">
          {t(
            '同步尚未配置。已收到的待处理材料仍保留在本机，配置连接后可继续处理。',
            'Sync is not configured. Previously received pending material remains on this device and can be handled after connecting.',
          )}
        </p>
      ) : props.configured === null ? (
        <p className="tag-sync-connection" role="status">
          {t(
            '尚未取得同步连接状态。可先查看本机保留的比较材料。',
            'The sync connection state is not available yet. You can review retained local material.',
          )}
        </p>
      ) : !props.canReadRemote ? (
        <p className="tag-sync-connection" role="status">
          {t(
            '当前无法读取远端。请先恢复连接或授权，比较材料仍保留在本机。',
            'The remote service is unavailable. Restore connectivity or authorization first; comparison material remains on this device.',
          )}
        </p>
      ) : readOnly ? (
        <p className="tag-sync-connection" role="status">
          {t(
            '此设备没有推送权限，可采用可用的远端版本；保留并推送本机需要编辑权限。',
            'This device cannot push. It may accept an available remote version; keeping and pushing local state requires edit permission.',
          )}
        </p>
      ) : null}
      {list.error && (
        <PageState
          kind="error"
          compact
          title={t('待处理记录读取失败', 'Could not read pending records')}
          detail={list.error}
          onRetry={refresh}
          retryLabel={t('重新读取', 'Reload')}
        />
      )}
      {feedback && (
        <div
          ref={feedbackElement}
          tabIndex={-1}
          className={`tag-sync-feedback is-${feedback.tone}`}
          role={feedback.tone === 'error' ? 'alert' : 'status'}
        >
          <AlertCircle size={16} aria-hidden="true" />
          <div>
            <p>{feedback.text}</p>
            {failedAction && (
              <button
                type="button"
                className="secondary-button"
                disabled={
                  busy || !tagSyncAvailability(failedAction.snapshot, props)[failedAction.mode]
                }
                onClick={() => {
                  if (failedAction.mode === 'retry') void act(failedAction.snapshot, 'retry');
                  else
                    setConfirmation({ snapshot: failedAction.snapshot, mode: failedAction.mode });
                }}
              >
                <RefreshCw size={14} aria-hidden="true" />
                {t('重试本次处理', 'Retry this action')}
              </button>
            )}
          </div>
        </div>
      )}
      {selected ? (
        <div className="tag-sync-detail">
          <button
            type="button"
            className="text-button tag-sync-back"
            disabled={busy}
            onClick={() => {
              const id = selectedId;
              setSelectedId(null);
              setInspection(null);
              setFailedAction(null);
              setHoldComparison(false);
              requestAnimationFrame(() => {
                const target = [
                  ...(listElement.current?.querySelectorAll<HTMLButtonElement>(
                    'button[data-sync-object-id]',
                  ) ?? []),
                ].find((button) => button.dataset.syncObjectId === id);
                (target ?? headingElement.current)?.focus({ preventScroll: true });
              });
            }}
          >
            <ArrowLeft size={15} aria-hidden="true" />
            {t('返回待处理列表', 'Back to pending records')}
          </button>
          <header>
            <p>
              {selected.kind === 'tag'
                ? t('标签定义', 'Tag definition')
                : t('内容的标签分配', 'Content tag assignments')}
            </p>
            <h4 ref={detailHeading} tabIndex={-1}>
              {tagSyncTitle(selected, t)}
            </h4>
          </header>
          {selected.pending && (
            <div className="tag-sync-reason">
              <strong>{tagSyncReason(selected.pending.reason, t).title}</strong>
              <p>{tagSyncReason(selected.pending.reason, t).detail}</p>
            </div>
          )}
          <div
            className="tag-sync-detail-body"
            ref={detailElement}
            onScroll={() => {
              if (detailElement.current) setDetailScroll(detailElement.current.scrollTop);
            }}
          >
            <TagSyncComparison
              snapshot={selected}
              onOpenView={busy ? undefined : props.onOpenView}
            />
          </div>
          <div className="tag-sync-help-actions">
            <button
              type="button"
              className="secondary-button"
              disabled={busy}
              onClick={props.onOpenTags}
            >
              <Tags size={15} aria-hidden="true" />
              {t('标签管理', 'Manage tags')}
            </button>
            {availability?.openItem && (
              <button
                type="button"
                className="secondary-button"
                disabled={busy}
                onClick={() => props.onOpenItem(selected)}
              >
                <ExternalLink size={15} aria-hidden="true" />
                {t('打开本机内容', 'Open local content')}
              </button>
            )}
            <button
              type="button"
              className="secondary-button"
              disabled={blocked || !availability?.retry}
              onClick={() => void act(freezeTagSyncDecision(selected), 'retry')}
            >
              <RefreshCw size={15} aria-hidden="true" />
              {t('重新检查', 'Check again')}
            </button>
          </div>
          <div className="tag-sync-decisions">
            <button
              type="button"
              className="secondary-button"
              disabled={blocked || !availability?.local}
              onClick={() => decide(selected, 'local')}
            >
              {t('保留本机', 'Keep local')}
            </button>
            <button
              type="button"
              className="primary-button"
              disabled={blocked || !availability?.remote}
              onClick={() => decide(selected, 'remote')}
            >
              {isDeletion(selected.pending?.payload)
                ? t('采用远端删除', 'Accept remote deletion')
                : t('采用远端', 'Use remote')}
            </button>
          </div>
          {busy && (
            <p className="tag-sync-muted" role="status">
              {t('正在核对并处理这条记录…', 'Checking and processing this record…')}
            </p>
          )}
        </div>
      ) : (
        <>
          <div className="tag-sync-list" ref={listElement} onScroll={paging.rememberScroll}>
            {(loading || (!list.page && !list.error)) && (
              <PageState
                kind="loading"
                compact
                title={t('正在读取待处理记录…', 'Reading pending records…')}
              />
            )}
            {!loading && !list.error && list.page?.total === 0 && (
              <p className="tag-sync-empty">
                {t('当前没有待处理的标签同步记录。', 'There are no pending tag sync records.')}
              </p>
            )}
            <ul>
              {list.page?.items.map((snapshot) => (
                <li key={snapshot.sync_object_id}>
                  <button
                    type="button"
                    data-sync-object-id={snapshot.sync_object_id}
                    disabled={loading || busy}
                    onClick={() => open(snapshot)}
                  >
                    <Hash size={18} aria-hidden="true" />
                    <span>
                      <strong>{tagSyncTitle(snapshot, t)}</strong>
                      <small>
                        {snapshot.pending && tagSyncReason(snapshot.pending.reason, t).title}
                      </small>
                    </span>
                    <ChevronRight size={16} aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
          </div>
          <div className="tag-sync-pager">
            <ListPagination list={pager} disabled={busy} />
          </div>
        </>
      )}
      {confirmation && (
        <ActionConfirmDialog
          title={
            confirmation.mode === 'local'
              ? t('确认保留本机状态？', 'Keep the local state?')
              : isDeletion(confirmation.snapshot.pending?.payload)
                ? t('确认采用远端删除？', 'Accept the remote deletion?')
                : t('确认采用远端状态？', 'Use the remote state?')
          }
          description={tagSyncConfirmation(confirmation.snapshot, confirmation.mode, t)}
          confirmLabel={
            confirmation.mode === 'local'
              ? t('保留本机', 'Keep local')
              : t('采用远端', 'Use remote')
          }
          tone={
            isDeletion(
              confirmation.mode === 'local'
                ? confirmation.snapshot.payload
                : confirmation.snapshot.pending?.payload,
            )
              ? 'danger'
              : 'default'
          }
          onCancel={() => setConfirmation(null)}
          onConfirm={() => {
            const decision = confirmation;
            setConfirmation(null);
            void act(decision.snapshot, decision.mode);
          }}
        />
      )}
    </section>
  );
}

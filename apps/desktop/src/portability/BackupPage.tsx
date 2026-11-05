import { Select } from '../ui/Select';
import { useEffect, useRef, useState } from 'react';
import {
  AlertCircle,
  Archive,
  ArrowDownToLine,
  ArrowLeftRight,
  ArrowUpFromLine,
  Check,
  Cloud,
  FileArchive,
  FolderOpen,
  HardDrive,
  Info,
  KeyRound,
  Link2,
  ListTodo,
  LoaderCircle,
  RefreshCw,
  ShieldCheck,
  Trash2,
  Wifi,
  WifiOff,
} from 'lucide-react';
import type {
  BackupInspection,
  ResourceGrant,
  SyncDevice,
  SyncInviteView,
  SyncAuditEvent,
  SyncMember,
  SyncMemberRemoval,
  SyncMemberRole,
  SyncOwnerProof,
  SyncPullView,
  SyncPushSummary,
  SyncStatus,
  TagSyncSnapshot,
  SyncTrashItem,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, choose, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { portableActions, usePortableActions } from './actions';
import { acceptedTaskDestination, type OpenTasks, type TaskDestination } from './task-navigation';
import {
  announceSyncPulled,
  announceSyncStatusChanged,
  SYNC_STATUS_CHANGED_EVENT,
} from './sync-status-events';
import {
  SyncConflictConfirmDialog,
  type SyncConflictResolution,
} from './SyncConflictConfirmDialog';
import { PageState } from '../layout/PageState';
import { PageFrame } from '../layout/PageFrame';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import './backup-page.css';
import { TagSyncIssues } from './TagSyncIssues';
import { announceTagChange, sameTagSyncScope, TAG_METADATA_CHANGED_EVENT } from './tag-sync-events';

export type BackupPageProps = {
  identity: WorkspaceIdentity | null;
  withSavedWorkspace: (identity: WorkspaceIdentity, action: () => Promise<void>) => Promise<void>;
  onTasks: OpenTasks;
  onHandoffImported: (noteIds: string[]) => void;
  onConfigurationChanged: () => Promise<void>;
  onOpenTagSyncItem: (snapshot: TagSyncSnapshot) => void;
  onOpenTags: () => void;
  onOpenView: (id: string) => void;
};
const identityKey = (identity: WorkspaceIdentity | null) =>
  identity ? `${identity.local_instance_id}/${identity.workspace_id}` : 'no-workspace';
type SelectedPackage = { grant: ResourceGrant; inspection: BackupInspection };
type Activity = 'select' | 'inspect' | 'create' | 'verify' | 'restore' | null;
type PendingConflict = {
  conflict: SyncPullView['server_conflicts'][number];
  mode: SyncConflictResolution;
};
type PendingConfigConflict = {
  conflict: SyncPullView['config_conflicts'][number];
  mode: 'local' | 'remote';
};

function configKindLabel(kind: string, t: (zh: string, en: string) => string) {
  return kind === 'canvas'
    ? t('画布', 'canvas')
    : kind === 'view'
      ? t('保存视图', 'saved view')
      : t('模板与对象类型', 'templates and object types');
}

function configConfirmation(pending: PendingConfigConflict, t: (zh: string, en: string) => string) {
  const label = configKindLabel(pending.conflict.kind, t);
  const deleting =
    pending.mode === 'local' ? pending.conflict.local_tombstone : pending.conflict.remote_tombstone;
  if (deleting)
    return pending.mode === 'local'
      ? t(
          `将本机对${label}的删除同步到其他设备。该配置记录将移除，其中引用的笔记保留。`,
          `Sync the local deletion of this ${label}. The configuration is removed; referenced notes are retained.`,
        )
      : t(
          `采用远端删除并移除本机的${label}记录。其中引用的笔记保留；本页无法撤销这次配置删除。`,
          `Use the remote deletion and remove this local ${label}. Referenced notes are retained; this page cannot undo the configuration deletion.`,
        );
  return pending.mode === 'local'
    ? t(
        `将本机的${label}作为新的当前版本推送，替换已核对的远端状态。其他设备下次拉取后采用该版本。`,
        `Push the local ${label} as the new current version, replacing the reviewed remote state. Other devices adopt it on their next pull.`,
      )
    : t(
        `将已核对的远端${label}写入本机并替换当前配置。这类配置不属于笔记版本历史，本页无法撤销。`,
        `Apply the reviewed remote ${label}, replacing the local configuration. It is outside note revision history and cannot be undone from this page.`,
      );
}

export function BackupPage(props: BackupPageProps) {
  // Drop selected grants and in-flight UI results when the active instance changes.
  return <BackupScope key={identityKey(props.identity)} {...props} />;
}
function BackupScope({
  identity,
  withSavedWorkspace,
  onTasks,
  onHandoffImported,
  onConfigurationChanged,
  onOpenTagSyncItem,
  onOpenTags,
  onOpenView,
}: BackupPageProps) {
  const { t, language } = useI18n();
  const requests = usePortableActions();
  const [selected, setSelected] = useState<SelectedPackage | null>(null);
  const [activity, setActivity] = useState<Activity>(null);
  const [error, setError] = useState<string | null>(null);
  const [accepted, setAccepted] = useState<TaskDestination | null>(null);
  const [handoffBusy, setHandoffBusy] = useState<'export' | 'import' | null>(null);
  const [handoffMessage, setHandoffMessage] = useState<string | null>(null);
  const [syncStatus, setSyncStatus] = useState<SyncStatus | null>(null);
  const [tagRefreshVersion, setTagRefreshVersion] = useState(0);
  const [syncDevices, setSyncDevices] = useState<SyncDevice[]>([]);
  const [syncInvite, setSyncInvite] = useState<SyncInviteView | null>(null);
  const [syncMembers, setSyncMembers] = useState<SyncMember[]>([]);
  const [syncAudit, setSyncAudit] = useState<SyncAuditEvent[]>([]);
  const [presenceNow, setPresenceNow] = useState(() => Date.now());
  const [inviteMemberName, setInviteMemberName] = useState('');
  const [pendingMemberRemoval, setPendingMemberRemoval] = useState<SyncMember | null>(null);
  const [inviteRole, setInviteRole] = useState<SyncMemberRole>('editor');
  const [syncOwnerProof, setSyncOwnerProof] = useState<SyncOwnerProof | null>(null);
  const [ownerDeviceId, setOwnerDeviceId] = useState('');
  const [syncPull, setSyncPull] = useState<SyncPullView | null>(null);
  const [pendingConfigConflicts, setPendingConfigConflicts] = useState<
    SyncPullView['config_conflicts']
  >([]);
  const [conflictConfirm, setConflictConfirm] = useState<PendingConflict | null>(null);
  const [configConflictConfirm, setConfigConflictConfirm] = useState<PendingConfigConflict | null>(
    null,
  );
  const [syncTrash, setSyncTrash] = useState<SyncTrashItem[]>([]);
  const [syncBusy, setSyncBusy] = useState<string | null>(null);
  const [syncMessage, setSyncMessage] = useState<string | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [syncLoadState, setSyncLoadState] = useState<'loading' | 'ready' | 'failed'>('loading');
  const [syncEndpoint, setSyncEndpoint] = useState('http://127.0.0.1:8787');
  const [syncDisplayName, setSyncDisplayName] = useState('这台电脑');
  const [pairInviteId, setPairInviteId] = useState('');
  const [pairCode, setPairCode] = useState('');
  const [inspectingName, setInspectingName] = useState('');
  const working = useRef(false);
  const alive = useRef(true);
  const selfSyncDevice = syncStatus?.device_id
    ? syncDevices.find((device) => device.device_id === syncStatus.device_id)
    : undefined;
  const canAdministerSync = selfSyncDevice?.role === 'owner';
  const canWriteSync = selfSyncDevice?.role === 'owner' || selfSyncDevice?.role === 'editor';
  const selectionSequence = useRef(0);
  const selectedToken = useRef<string | null>(null);
  useEffect(() => {
    alive.current = true;
    void refreshSync();
    return () => {
      alive.current = false;
      ++selectionSequence.current;
    };
  }, []);
  useEffect(() => {
    const refresh = () => {
      void loadPendingConfigConflicts();
      setTagRefreshVersion((version) => version + 1);
    };
    const tagsChanged = (event: Event) => {
      if (sameTagSyncScope((event as CustomEvent<WorkspaceIdentity>).detail, identity))
        setTagRefreshVersion((version) => version + 1);
    };
    window.addEventListener(SYNC_STATUS_CHANGED_EVENT, refresh);
    window.addEventListener(TAG_METADATA_CHANGED_EVENT, tagsChanged);
    return () => {
      window.removeEventListener(SYNC_STATUS_CHANGED_EVENT, refresh);
      window.removeEventListener(TAG_METADATA_CHANGED_EVENT, tagsChanged);
    };
  }, [identity]);
  useEffect(() => {
    const timer = setInterval(() => setPresenceNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);
  const unresolved = requests.pending.filter(
    (action) =>
      action.command.kind === 'backup_verify' ||
      action.command.kind === 'backup_restore' ||
      (action.command.kind === 'backup_create' &&
        identityKey(action.identity) === identityKey(identity)),
  );
  const syncAttached =
    !!identity && !!syncStatus?.workspace_id && syncStatus.workspace_id === identity.workspace_id;
  const cannotSubmit = !!activity || !requests.loaded || unresolved.length > 0;
  const current = (sequence: number) => alive.current && sequence === selectionSequence.current;

  async function loadPendingConfigConflicts() {
    if (!identity) {
      setPendingConfigConflicts([]);
      return;
    }
    try {
      const conflicts = await call(
        { kind: 'sync_config_conflicts' },
        'sync_config_conflicts',
        identity,
        4_000,
      );
      if (alive.current) setPendingConfigConflicts(conflicts);
    } catch {
      if (alive.current) setPendingConfigConflicts([]);
    }
  }
  function applyPullResult(result: SyncPullView) {
    setSyncPull(result);
    setPendingConfigConflicts(result.config_conflicts);
    setTagRefreshVersion((version) => version + 1);
  }
  function validateGrant(
    grant: ResourceGrant,
    purpose: ResourceGrant['purpose'],
    expected: WorkspaceIdentity | null,
  ) {
    if (grant.purpose !== purpose || identityKey(grant.identity) !== identityKey(expected))
      throw new Error(
        t(
          '所选位置不属于这次操作，请重新选择。',
          'The selected location does not belong to this operation. Select it again.',
        ),
      );
    if (
      !Number.isFinite(Date.parse(grant.expires_at)) ||
      Date.parse(grant.expires_at) <= Date.now()
    )
      throw new Error(
        t(
          '所选文件或位置已过期，请重新选择。',
          'The file or location selection has expired. Select it again.',
        ),
      );
  }
  async function loadSync() {
    const status = await call({ kind: 'sync_status' }, 'sync_status', null, 4_000);
    setSyncStatus(status);
    setSyncLoadState('ready');
    if (!status.configured || !status.device_id) {
      setSyncDevices([]);
      setSyncMembers([]);
      setSyncAudit([]);
      setPendingConfigConflicts([]);
      setSyncTrash([]);
      return;
    }
    const [devices, members] = await Promise.all([
      call({ kind: 'sync_devices' }, 'sync_devices', null, 4_000),
      call({ kind: 'sync_members' }, 'sync_members', null, 4_000),
    ]);
    setSyncDevices(devices);
    setSyncMembers(members);
    if (identity && status.workspace_id === identity.workspace_id) {
      await loadPendingConfigConflicts();
    } else {
      setPendingConfigConflicts([]);
    }
    const self = devices.find((device) => device.device_id === status.device_id);
    if (self?.role === 'owner') {
      try {
        setSyncAudit(
          await call({ kind: 'sync_audit', payload: { limit: 50 } }, 'sync_audit', null, 4_000),
        );
      } catch {
        // Audit is an owner-only diagnostic view; a transient failure must not
        // hide the member and device controls.
        setSyncAudit([]);
      }
    } else {
      setSyncAudit([]);
    }
    try {
      const trash = await call({ kind: 'sync_trash' }, 'sync_trash', identity, 4_000);
      setSyncTrash(trash);
    } catch (failure) {
      // A reachable authority with no attached workspace still has a valid
      // device list. Keep it visible instead of hiding the whole panel.
      setSyncTrash([]);
      setSyncError(message(failure));
    }
  }
  async function refreshSync() {
    if (syncBusy) return;
    setSyncBusy('refresh');
    setSyncError(null);
    setSyncLoadState('loading');
    try {
      await loadSync();
    } catch (failure) {
      setSyncError(message(failure));
      setSyncLoadState('failed');
    } finally {
      setSyncBusy(null);
      announceSyncStatusChanged();
    }
  }
  async function syncAction(label: string, action: () => Promise<void>) {
    if (syncBusy) return;
    setSyncBusy(label);
    setSyncError(null);
    setSyncMessage(null);
    try {
      await action();
    } catch (failure) {
      setSyncError(message(failure));
    } finally {
      setSyncBusy(null);
      announceSyncStatusChanged();
    }
  }
  function configureSync() {
    void syncAction('configure', async () => {
      await call(
        {
          kind: 'sync_configure',
          payload: { endpoint: syncEndpoint.trim(), display_name: syncDisplayName.trim() },
        },
        'sync_profile',
      );
      await loadSync();
      setSyncMessage(
        t(
          '同步身份已保存在本机凭据库。下一步生成配对码，或输入另一台设备的配对码。',
          'The sync identity is in the local credential vault. Create a pairing code or enter one from another device.',
        ),
      );
    });
  }
  function roleLabel(role: SyncMemberRole) {
    if (role === 'owner') return t('所有者', 'Owner');
    if (role === 'editor') return t('编辑者', 'Editor');
    return t('只读者', 'Viewer');
  }
  function auditActionLabel(action: string) {
    const labels: Record<string, [string, string]> = {
      owner_enrolled: ['所有者设备已加入', 'Owner device enrolled'],
      pairing_invite_created: ['创建了配对码', 'Pairing code created'],
      device_paired: ['设备已配对', 'Device paired'],
      device_revoked: ['设备已撤销', 'Device revoked'],
      member_role_changed: ['成员角色已更改', 'Member role changed'],
      member_removed: ['成员已移除', 'Member removed'],
      workspace_key_rotated: ['同步密钥已轮换', 'Sync key rotated'],
      workspace_key_granted: ['同步密钥已授权给设备', 'Sync key granted to device'],
      revoke_refused_last_active: ['拒绝撤销最后一台设备', 'Refused to revoke the last device'],
      revoke_refused_last_owner: ['拒绝撤销最后一个所有者', 'Refused to revoke the last owner'],
      access_denied_revoked: ['拒绝已撤销设备访问', 'Denied access for a revoked device'],
      access_denied_invalid_signature: [
        '拒绝签名无效的请求',
        'Denied a request with an invalid signature',
      ],
      access_denied_role: ['拒绝权限不足的请求', 'Denied a request with insufficient permission'],
      access_denied_replayed: ['拒绝重放请求', 'Denied a replayed request'],
    };
    return labels[action]?.[language === 'en' ? 1 : 0] ?? action;
  }
  function auditDetailText(entry: SyncAuditEvent) {
    if (!entry.detail || typeof entry.detail !== 'object') return '';
    const detail = entry.detail as Record<string, unknown>;
    const name = detail.member_name ?? detail.display_name;
    const role = detail.role ?? detail.to;
    return [name, role].filter((value) => typeof value === 'string' && value).join(' · ');
  }
  function deviceIsOnline(device: SyncDevice) {
    if (device.status !== 'active' || !device.last_seen_at) return false;
    const seen = Date.parse(device.last_seen_at);
    return Number.isFinite(seen) && presenceNow - seen < 120_000;
  }
  function createSyncInvite() {
    const memberName = inviteMemberName.trim();
    if (!memberName) return;
    void syncAction('invite', async () => {
      const invite = await call(
        {
          kind: 'sync_create_invite',
          payload: { role: inviteRole, member_name: memberName },
        },
        'sync_invite',
      );
      setSyncInvite(invite);
      setInviteMemberName('');
      setSyncMessage(
        t(
          `配对码 10 分钟内有效；${memberName}加入后角色为${roleLabel(invite.role)}。`,
          `The pairing code is valid for 10 minutes; ${memberName} joins as ${roleLabel(invite.role)}.`,
        ),
      );
    });
  }
  function changeSyncMemberRole(memberId: string, role: SyncMemberRole) {
    void syncAction('role', async () => {
      await call(
        {
          kind: 'sync_set_member_role',
          payload: { member_id: memberId, role },
        },
        'sync_member',
      );
      await loadSync();
      const label = roleLabel(role);
      setSyncMessage(t(`成员角色已更新为${label}。`, `The member role is now ${label}.`));
    });
  }
  function inviteMemberDevice(member: SyncMember) {
    void syncAction('device-invite', async () => {
      const invite = await call(
        {
          kind: 'sync_create_device_invite',
          payload: { member_id: member.member_id, role: member.role },
        },
        'sync_invite',
      );
      setSyncInvite(invite);
      setSyncMessage(
        t(
          `已生成${member.display_name}的新设备配对码，10 分钟内有效。`,
          `A new device pairing code for ${member.display_name} is valid for 10 minutes.`,
        ),
      );
    });
  }
  function removeSyncMember(member: SyncMember) {
    void syncAction('remove-member', async () => {
      const removal: SyncMemberRemoval = await call(
        {
          kind: 'sync_remove_member',
          payload: { member_id: member.member_id, reason: '所有者移除成员' },
        },
        'sync_member_removal',
      );
      setPendingMemberRemoval(null);
      await loadSync();
      const messageText = removal.key_rotated
        ? t(
            `${member.display_name}已被移除，其设备已撤销；同步密钥已轮换到第 ${removal.key_epoch} 代。`,
            `${member.display_name} was removed; their devices are revoked and the sync key rotated to epoch ${removal.key_epoch}.`,
          )
        : removal.rotation_error
          ? t(
              `${member.display_name}已被移除，但同步密钥轮换失败：${removal.rotation_error}。请点击“创建/轮换同步密钥”立即重试。`,
              `${member.display_name} was removed, but key rotation failed: ${removal.rotation_error}. Choose “Create/rotate sync key” now.`,
            )
          : t(
              `${member.display_name}已被移除，其设备已撤销且不能再读写。绑定知识库后请轮换同步密钥。`,
              `${member.display_name} was removed; their devices are revoked. Attach a library and rotate the sync key.`,
            );
      setSyncMessage(messageText);
    });
  }
  function pairSyncDevice() {
    if (!identity) return;
    void syncAction('pair', async () => {
      await call(
        {
          kind: 'sync_pair',
          payload: { invite_id: pairInviteId.trim(), code: pairCode.trim() },
        },
        'sync_profile',
      );
      setPairInviteId('');
      setPairCode('');
      await loadSync();
      setSyncMessage(
        t(
          '设备已加入同步。请已在同步中的设备点击“授权密钥”，然后再绑定当前知识库。',
          'The device joined sync. On an already-synced device choose “Grant key”, then attach this library.',
        ),
      );
    });
  }
  function loadOwnerProof() {
    void syncAction('owner-proof', async () => {
      setSyncOwnerProof(await call({ kind: 'sync_owner_proof' }, 'sync_owner_proof'));
    });
  }
  function claimOwnerDevice() {
    void syncAction('claim-owner', async () => {
      await call(
        { kind: 'sync_claim_device', payload: { device_id: ownerDeviceId.trim() } },
        'sync_profile',
      );
      setOwnerDeviceId('');
      await loadSync();
      setSyncMessage(
        t(
          '本机已作为服务器首台设备绑定。下一步创建同步密钥。',
          'This device is now bound as the server owner. Create the sync key next.',
        ),
      );
    });
  }
  function attachSyncWorkspace() {
    if (!identity) return;
    void syncAction('attach', async () => {
      await call(
        {
          kind: 'sync_attach_workspace',
          payload: { workspace_id: identity.workspace_id },
        },
        'sync_profile',
      );
      await loadSync();
      setSyncMessage(t('当前知识库已绑定到同步。', 'This library is now attached to sync.'));
    });
  }
  function grantSyncKey(deviceId: string) {
    if (!identity) return;
    void syncAction('grant', async () => {
      await call(
        {
          kind: 'sync_grant_workspace_key',
          payload: { workspace_id: identity.workspace_id, target_device_id: deviceId },
        },
        'done',
      );
      setSyncMessage(
        t(
          '已把当前及历史同步密钥授权给该设备。',
          'Current and historical sync keys were granted to this device.',
        ),
      );
    });
  }
  function ensureWorkspaceKey() {
    if (!identity) return;
    void syncAction('key', async () => {
      await call(
        { kind: 'sync_ensure_workspace_key', payload: { workspace_id: identity.workspace_id } },
        'sync_profile',
      );
      await loadSync();
      setSyncMessage(
        t(
          '同步密钥已创建/轮换，当前活动设备均已获得新信封。',
          'The sync key was created/rotated and every active device received a new envelope.',
        ),
      );
    });
  }
  function pushAllNotes() {
    if (!identity) return;
    void syncAction('push', async () => {
      const batch = await call({ kind: 'sync_push_all' }, 'sync_push_all', identity);
      const summaries = batch.items;
      setTagRefreshVersion((version) => version + 1);
      if (batch.tags.changed_objects.length)
        announceTagChange(TAG_METADATA_CHANGED_EVENT, identity);
      const conflicts = summaries.filter(
        (item: SyncPushSummary) => item.conflict_remote_version !== null,
      ).length;
      const pushed = summaries.filter(
        (item: SyncPushSummary) => !item.unchanged && item.conflict_remote_version === null,
      ).length;
      if (conflicts > 0) {
        const result = await call({ kind: 'sync_pull' }, 'sync_pull', identity);
        applyPullResult(result);
        announceSyncPulled(result.applied, identity);
        await loadSync();
        setSyncMessage(
          t(
            `推送发现 ${conflicts} 条冲突；已载入并排比较，未覆盖任何一方。`,
            `${conflicts} conflict(s) found; side-by-side comparison loaded and neither side was overwritten.`,
          ),
        );
      } else {
        setSyncMessage(
          t(
            `已推送 ${pushed} 项内容或配置，${summaries.length - pushed} 项未变化。标签与分配：推送 ${batch.tags.published}，未变化 ${batch.tags.unchanged}，等待处理 ${batch.tags.pending}，仍有本机修改 ${batch.tags.dirty}。`,
            `Pushed ${pushed} content/configuration item(s); ${summaries.length - pushed} unchanged. Tags and assignments: ${batch.tags.published} pushed, ${batch.tags.unchanged} unchanged, ${batch.tags.pending} pending, ${batch.tags.dirty} with local changes.`,
          ),
        );
      }
    });
  }
  function pullNotes() {
    if (!identity) return;
    void syncAction('pull', async () => {
      const result = await call({ kind: 'sync_pull' }, 'sync_pull', identity);
      applyPullResult(result);
      announceSyncPulled(result.applied, identity);
      await loadSync();
      setSyncMessage(
        t(
          `本批拉取：应用 ${result.applied.length}，跳过 ${result.skipped.length}，内容或配置冲突 ${result.server_conflicts.length + result.conflicts.length + result.config_conflicts.length}，标签等待处理 ${result.tag_pending}。后续批次会继续自动接收。`,
          `This pull batch: ${result.applied.length} applied, ${result.skipped.length} skipped, ${result.server_conflicts.length + result.conflicts.length + result.config_conflicts.length} content/configuration conflict(s), ${result.tag_pending} pending tag item(s). Further batches are received automatically.`,
        ),
      );
    });
  }
  function resolveConflict(
    target: SyncPullView['server_conflicts'][number]['target'],
    resolution: 'local' | 'remote' | 'new',
  ) {
    if (!identity) return;
    void syncAction('resolve', async () => {
      if (resolution === 'local')
        await call(
          { kind: 'sync_resolve_keep_local', payload: { conflict: target } },
          'sync_push',
          identity,
        );
      else if (resolution === 'remote')
        await call(
          { kind: 'sync_resolve_keep_remote', payload: { conflict: target } },
          'done',
          identity,
        );
      else
        await call(
          { kind: 'sync_resolve_save_as_new', payload: { conflict: target } },
          'sync_object_id',
          identity,
        );
      const result = await call({ kind: 'sync_pull' }, 'sync_pull', identity);
      applyPullResult(result);
      announceSyncPulled(result.applied, identity);
      await loadSync();
      setSyncMessage(t('冲突已处理。', 'The conflict was resolved.'));
    });
  }
  function resolveConfigConflict(
    conflict: SyncPullView['config_conflicts'][number],
    resolution: 'local' | 'remote',
  ) {
    if (!identity) return;
    void syncAction('resolve-config', async () => {
      if (resolution === 'local') {
        await call(
          {
            kind: 'sync_config_keep_local',
            payload: {
              object_id: conflict.object_id,
              remote_version: conflict.remote_version,
              local_state_token: conflict.local_state_token,
            },
          },
          'sync_config_resolution',
          identity,
        );
      } else {
        await call(
          {
            kind: 'sync_config_keep_remote',
            payload: {
              object_id: conflict.object_id,
              remote_version: conflict.remote_version,
              local_state_token: conflict.local_state_token,
            },
          },
          'sync_config_resolution',
          identity,
        );
        await onConfigurationChanged();
      }
      const result = await call({ kind: 'sync_pull' }, 'sync_pull', identity);
      applyPullResult(result);
      announceSyncPulled(result.applied, identity);
      await loadSync();
      setSyncMessage(
        resolution === 'local'
          ? t(
              '本机配置已作为当前版本推送；远端版本已被这次明确选择替换。',
              'The local configuration was pushed as the current version; the remote version was explicitly replaced.',
            )
          : t(
              '已采用核对后的远端配置，相关页面已刷新。',
              'The reviewed remote configuration was applied and the related pages were refreshed.',
            ),
      );
    });
  }
  function revokeSyncDevice(deviceId: string) {
    void syncAction('revoke', async () => {
      await call(
        { kind: 'sync_revoke_device', payload: { target_device_id: deviceId, reason: '用户撤销' } },
        'sync_device',
      );
      await loadSync();
      setSyncMessage(
        t(
          '设备已撤销，之后无法继续读写。',
          'The device was revoked and can no longer read or write.',
        ),
      );
    });
  }
  function restoreSyncTrash(objectId: string) {
    if (!identity) return;
    void syncAction('restore', async () => {
      await call(
        { kind: 'sync_restore_note', payload: { note_id: objectId } },
        'sync_push',
        identity,
      );
      await loadSync();
      setSyncMessage(t('已恢复并推送。', 'Restored and pushed.'));
    });
  }
  function begin(next: Activity) {
    if (working.current) return false;
    working.current = true;
    setActivity(next);
    setError(null);
    setAccepted(null);
    return true;
  }
  function finish() {
    working.current = false;
    if (alive.current) setActivity(null);
  }
  async function selectBackup() {
    if (!begin('select')) return;
    const sequence = ++selectionSequence.current;
    selectedToken.current = null;
    setSelected(null);
    setInspectingName('');
    try {
      const grant = await choose('backup_input', identity);
      if (!grant || !current(sequence)) return;
      validateGrant(grant, 'backup_input', null);
      selectedToken.current = grant.token;
      setInspectingName(grant.display_name);
      setActivity('inspect');
      const inspection = await call(
        { kind: 'backup_inspect', payload: { token: grant.token } },
        'backup_inspection',
      );
      if (!current(sequence) || selectedToken.current !== grant.token) return;
      setSelected({ grant, inspection });
    } catch (error) {
      if (current(sequence)) {
        setError(message(error));
        selectedToken.current = null;
      }
    } finally {
      finish();
    }
  }
  async function createBackup() {
    if (!identity || cannotSubmit || !begin('create')) return;
    const scope = identity;
    let acceptedTask: TaskDestination | null = null;
    try {
      const destination = await choose('export_file', scope);
      if (!destination || !alive.current) return;
      validateGrant(destination, 'export_file', scope);
      const id = crypto.randomUUID();
      await withSavedWorkspace(scope, async () => {
        if (!alive.current)
          throw new Error(
            t(
              '工作区已切换，请在当前工作区重新操作。',
              'The workspace changed. Start again in the current workspace.',
            ),
          );
        validateGrant(destination, 'export_file', scope);
        const result = await portableActions.submit({
          id,
          identity: scope,
          command: { kind: 'backup_create', payload: { command_id: id, token: destination.token } },
          displayName: destination.display_name,
          createdAt: new Date().toISOString(),
        });
        acceptedTask = acceptedTaskDestination(result);
      });
      if (alive.current) setAccepted(acceptedTask);
    } catch (error) {
      if (alive.current) setError(message(error));
    } finally {
      finish();
      if (acceptedTask && alive.current) onTasks(acceptedTask);
    }
  }
  async function exportHandoff() {
    if (!identity || handoffBusy) return;
    const scope = identity;
    setHandoffBusy('export');
    setHandoffMessage(null);
    setError(null);
    try {
      const destination = await choose('handoff_output', scope);
      if (!destination) return;
      validateGrant(destination, 'handoff_output', scope);
      const result = await call(
        { kind: 'handoff_export', payload: { token: destination.token } },
        'handoff_export',
        scope,
      );
      setHandoffMessage(
        t(`已导出 ${result.objects} 条续读内容。`, `Exported ${result.objects} resume item(s).`),
      );
    } catch (error) {
      setError(message(error));
    } finally {
      setHandoffBusy(null);
    }
  }

  async function importHandoff() {
    if (!identity || handoffBusy) return;
    const scope = identity;
    setHandoffBusy('import');
    setHandoffMessage(null);
    setError(null);
    try {
      const source = await choose('handoff_input', scope);
      if (!source) return;
      validateGrant(source, 'handoff_input', scope);
      const result = await call(
        { kind: 'handoff_import', payload: { token: source.token } },
        'handoff_import',
        scope,
      );
      if (result.imported.length > 0) onHandoffImported(result.imported);
      setHandoffMessage(
        t(
          `已导入 ${result.imported.length} 条，跳过已有 ${result.skipped_existing.length} 条。`,
          `Imported ${result.imported.length}; skipped ${result.skipped_existing.length} existing item(s).`,
        ),
      );
    } catch (error) {
      setError(message(error));
    } finally {
      setHandoffBusy(null);
    }
  }
  async function useBackup(kind: 'verify' | 'restore') {
    if (!selected || cannotSubmit || !begin(kind)) return;
    const source = selected;
    const sequence = selectionSequence.current;
    let acceptedTask: TaskDestination | null = null;
    try {
      validateGrant(source.grant, 'backup_input', null);
      let destination: ResourceGrant | null = null;
      if (kind === 'restore') {
        destination = await choose('restore_workspace', identity);
        if (!destination || !current(sequence) || selectedToken.current !== source.grant.token)
          return;
        validateGrant(destination, 'restore_workspace', null);
      }
      if (!current(sequence) || selectedToken.current !== source.grant.token) return;
      validateGrant(source.grant, 'backup_input', null);
      const id = crypto.randomUUID();
      const result = await portableActions.submit({
        id,
        identity: null,
        command:
          kind === 'verify'
            ? { kind: 'backup_verify', payload: { command_id: id, token: source.grant.token } }
            : {
                kind: 'backup_restore',
                payload: {
                  command_id: id,
                  token: source.grant.token,
                  output_token: destination!.token,
                },
              },
        displayName: source.grant.display_name,
        createdAt: new Date().toISOString(),
      });
      acceptedTask = acceptedTaskDestination(result);
      if (current(sequence)) setAccepted(acceptedTask);
    } catch (error) {
      if (current(sequence)) setError(message(error));
    } finally {
      finish();
      if (acceptedTask && alive.current) onTasks(acceptedTask);
    }
  }
  const busyText =
    activity === 'inspect'
      ? t('正在读取备份包信息…', 'Reading backup package details…')
      : activity === 'select'
        ? t('请选择备份包。', 'Choose a backup package.')
        : activity === 'create'
          ? t('正在选择保存位置并准备备份…', 'Choosing a destination and preparing the backup…')
          : activity === 'verify'
            ? t('正在提交完整校验请求…', 'Submitting full verification…')
            : t('正在选择新目录并提交恢复请求…', 'Choosing a new folder and submitting restore…');
  return (
    <PageFrame
      className="backup-page"
      title={t('备份与恢复', 'Backup and restore')}
      description={t(
        '把完整的知识库带走，也能从备份重新开始。',
        'Keep a complete copy of your library, and restore it when you need it.',
      )}
      actions={
        <button className="secondary-button" onClick={() => onTasks(accepted ?? undefined)}>
          <ListTodo size={16} />
          {t('查看任务', 'View tasks')}
        </button>
      }
    >
      {!requests.loaded && (
        <div className="backup-feedback" role={requests.error ? 'alert' : 'status'}>
          {requests.error ? (
            <>
              <AlertCircle size={17} />
              <div>
                <p>{requests.error}</p>
                <button
                  className="text-button"
                  onClick={() =>
                    void portableActions.load().catch((error) => {
                      if (alive.current) setError(message(error));
                    })
                  }
                >
                  {t('重新读取请求记录', 'Reload request history')}
                </button>
              </div>
            </>
          ) : (
            <>
              <LoaderCircle size={16} className="spin" />
              <span>{t('正在读取待确认请求…', 'Loading unconfirmed requests…')}</span>
            </>
          )}
        </div>
      )}
      {unresolved.length > 0 && !activity && (
        <div className="backup-feedback attention" role="alert">
          <AlertCircle size={18} />
          <div>
            <strong>{t('有备份请求尚未确认', 'A backup request still needs confirmation')}</strong>
            <p>
              {t(
                '原请求已保留，请到任务中心重查同一次操作的结果，再发起新的请求。',
                'The original request is kept. Confirm that operation in Tasks before submitting another request.',
              )}
            </p>
            <button className="secondary-button" onClick={() => onTasks(accepted ?? undefined)}>
              {t('前往任务中心', 'Open task center')}
            </button>
          </div>
        </div>
      )}
      {error && (
        <div className="backup-feedback attention" role="alert">
          <AlertCircle size={18} />
          <span>{error}</span>
        </div>
      )}
      {activity && (
        <div className="backup-feedback" role="status">
          <LoaderCircle size={16} className="spin" />
          <span>{busyText}</span>
        </div>
      )}
      {accepted && (
        <div className="backup-feedback" role="status">
          <Check size={16} />
          <span>
            {t(
              '请求已受理，实际执行结果请在任务中心查看。',
              'The request was accepted. Check Tasks for its execution result.',
            )}
          </span>
        </div>
      )}
      <div className="backup-sections">
        <section className="backup-section">
          <div className="backup-section-icon">
            <ArrowLeftRight size={22} />
          </div>
          <div className="backup-section-content">
            <h2>{t('跨设备续读', 'Cross-device resume')}</h2>
            <p>
              {t(
                '导出当前知识库为续读包，或导入手机/另一台设备导出的续读包。导入不会覆盖同 ID 但正文不同的对象。',
                'Export this library as a resume handoff, or import one from your phone or another device. Import never overwrites an existing ID with different text.',
              )}
            </p>
            <div className="row-actions">
              <button
                className="secondary-button"
                disabled={!identity || !!handoffBusy}
                onClick={() => void exportHandoff()}
              >
                <ArrowDownToLine size={17} />
                {t('导出续读包', 'Export resume handoff')}
              </button>
              <button
                className="secondary-button"
                disabled={!identity || !!handoffBusy}
                onClick={() => void importHandoff()}
              >
                <ArrowUpFromLine size={17} />
                {t('导入续读包', 'Import resume handoff')}
              </button>
            </div>
            {handoffBusy && (
              <p className="backup-small" role="status">
                <LoaderCircle size={14} className="spin" />
                {handoffBusy === 'export'
                  ? t('正在选择位置并导出…', 'Choosing a destination and exporting…')
                  : t('正在导入并校验续读包…', 'Importing and validating the handoff…')}
              </p>
            )}
            {handoffMessage && (
              <p className="backup-small" role="status">
                <Check size={14} /> {handoffMessage}
              </p>
            )}
          </div>
        </section>
        <section className="backup-section">
          <div className="backup-section-icon">
            <Cloud size={22} />
          </div>
          <div className="backup-section-content">
            <h2>{t('实时同步', 'Live sync')}</h2>
            <p>
              {t(
                '设备身份保存在 Windows 凭据库。配对后可推送当前库的笔记、拉取远端修改，并在并排比较后解决冲突。',
                'The device identity lives in Windows Credential Manager. Pair devices, push notes, pull remote edits, and resolve conflicts side by side.',
              )}
            </p>
            {syncLoadState === 'failed' && syncStatus === null ? (
              <PageState
                className="backup-state"
                compact
                kind="error"
                title={t('无法读取同步状态', 'Could not read sync status')}
                detail={syncError}
                retryLabel={t('重新读取同步状态', 'Reload sync status')}
                onRetry={() => void refreshSync()}
              />
            ) : syncStatus === null ? (
              <p className="backup-small" role="status">
                <LoaderCircle size={14} className="spin" />
                {t('正在读取同步状态…', 'Reading sync status…')}
              </p>
            ) : !syncStatus.configured ? (
              <div className="sync-form">
                <label>
                  <span>{t('同步服务器地址', 'Sync server address')}</span>
                  <input
                    value={syncEndpoint}
                    onChange={(event) => setSyncEndpoint(event.target.value)}
                    placeholder="http://127.0.0.1:8787"
                    spellCheck={false}
                  />
                </label>
                <label>
                  <span>{t('本机名称', 'This device')}</span>
                  <input
                    value={syncDisplayName}
                    onChange={(event) => setSyncDisplayName(event.target.value)}
                    maxLength={80}
                  />
                </label>
                <button className="primary-button" disabled={!!syncBusy} onClick={configureSync}>
                  <KeyRound size={17} />
                  {t('保存并连接', 'Save and connect')}
                </button>
              </div>
            ) : !syncStatus.device_id ? (
              <>
                <div className={`sync-status ${syncStatus.healthy ? 'online' : 'offline'}`}>
                  {syncStatus.healthy ? <Wifi size={16} /> : <WifiOff size={16} />}
                  <span>
                    {syncStatus.healthy
                      ? t(
                          `服务器可达，但本机尚未加入设备列表。`,
                          `The server is reachable, but this device has not joined yet.`,
                        )
                      : t(
                          `服务器当前不可达；本机身份已保存在凭据库，稍后可重试配对。`,
                          `The server is unreachable; this device identity is already in the credential vault for a later pairing retry.`,
                        )}
                  </span>
                </div>
                <div className="sync-pair">
                  <input
                    value={pairInviteId}
                    onChange={(event) => setPairInviteId(event.target.value)}
                    placeholder={t('配对 ID', 'Invite ID')}
                    spellCheck={false}
                  />
                  <input
                    value={pairCode}
                    onChange={(event) => setPairCode(event.target.value)}
                    placeholder={t('12 位配对码', '12-character code')}
                    spellCheck={false}
                  />
                  <button
                    className="secondary-button"
                    disabled={!pairInviteId.trim() || !pairCode.trim() || !!syncBusy}
                    onClick={pairSyncDevice}
                  >
                    {t('加入同步', 'Join sync')}
                  </button>
                </div>
                <details className="sync-owner">
                  <summary>
                    {t('首台设备注册（owner）', 'Register the first device (owner)')}
                  </summary>
                  <p className="backup-small">
                    {t(
                      '只有服务器上还没有 owner 时使用。生成材料后，在运行同步服务的机器上执行 bootstrap-owner，并把返回的 device ID 填回这里。',
                      'Use this only before the server has an owner. Generate the proof, run bootstrap-owner on the sync host, then paste the returned device ID here.',
                    )}
                  </p>
                  <button
                    className="secondary-button"
                    disabled={!!syncBusy}
                    onClick={loadOwnerProof}
                  >
                    <KeyRound size={16} />
                    {t('生成 owner 注册材料', 'Generate owner proof')}
                  </button>
                  {syncOwnerProof && (
                    <div className="sync-owner-proof">
                      <p>
                        <strong>{t('公钥', 'Public key')}</strong>
                        <code>{syncOwnerProof.public_key}</code>
                      </p>
                      <p>
                        <strong>{t('名称', 'Display name')}</strong>
                        <code>{syncOwnerProof.display_name}</code>
                      </p>
                      <p>
                        <strong>{t('proof', 'Proof')}</strong>
                        <code>{syncOwnerProof.proof}</code>
                      </p>
                      <p className="backup-small">
                        <code>
                          pkms-sync-server bootstrap-owner --db &lt;server.db&gt; --public-key{' '}
                          {syncOwnerProof.public_key} --display-name "{syncOwnerProof.display_name}"{' '}
                          --proof {syncOwnerProof.proof}
                        </code>
                      </p>
                    </div>
                  )}
                  <div className="sync-pair">
                    <input
                      value={ownerDeviceId}
                      onChange={(event) => setOwnerDeviceId(event.target.value)}
                      placeholder={t(
                        'bootstrap-owner 返回的 device ID',
                        'device ID returned by bootstrap-owner',
                      )}
                      spellCheck={false}
                    />
                    <button
                      className="secondary-button"
                      disabled={!ownerDeviceId.trim() || !!syncBusy}
                      onClick={claimOwnerDevice}
                    >
                      {t('完成首台设备绑定', 'Bind owner device')}
                    </button>
                  </div>
                </details>
                <p className="backup-small">
                  <Info size={14} />
                  {t(
                    '配对码由已加入同步的设备生成。已有 owner 时，直接输入配对码即可。',
                    'A paired device creates the code. If an owner already exists, just enter the pairing code.',
                  )}
                </p>
              </>
            ) : (
              <>
                <div className={`sync-status ${syncStatus.healthy ? 'online' : 'offline'}`}>
                  {syncStatus.healthy ? <Wifi size={16} /> : <WifiOff size={16} />}
                  <span>
                    {syncStatus.healthy
                      ? t(
                          `已连接 ${syncStatus.endpoint} · 协议 ${syncStatus.protocol_version} · 库 schema ${syncStatus.schema_version} · 设备 ${syncStatus.device_id.slice(0, 8)}`,
                          `Connected to ${syncStatus.endpoint} · protocol ${syncStatus.protocol_version} · library schema ${syncStatus.schema_version} · device ${syncStatus.device_id.slice(0, 8)}`,
                        )
                      : t(
                          `未连接 ${syncStatus.endpoint}，可稍后刷新或继续离线编辑。`,
                          `Not connected to ${syncStatus.endpoint}; keep editing offline and refresh later.`,
                        )}
                  </span>
                </div>
                {selfSyncDevice && !canAdministerSync && (
                  <p className="backup-small sync-role-note">
                    <Info size={14} />
                    {selfSyncDevice.role === 'viewer'
                      ? t(
                          '当前设备是只读成员：可拉取和阅读，但本地变更不会被推送。',
                          'This device is a viewer: it can pull and read, but local changes will not be pushed.',
                        )
                      : t(
                          '当前设备是编辑者：可编辑和同步，只有所有者能邀请成员、管理角色与密钥。',
                          'This device is an editor: editing and sync are available; only an owner can invite members and manage roles or keys.',
                        )}
                  </p>
                )}{' '}
                {!syncAttached && (
                  <div className="sync-status offline">
                    <Info size={16} />
                    <span>
                      {!identity
                        ? t(
                            '先打开一个知识库，才能把它绑定到同步。',
                            'Open a library before attaching it to sync.',
                          )
                        : t(
                            '当前知识库尚未绑定到同步。绑定前不会读取或推送正文。',
                            'The current library is not attached to sync. No note body is read or pushed until it is attached.',
                          )}
                    </span>
                    {identity && (
                      <>
                        <button
                          className="secondary-button"
                          disabled={!!syncBusy}
                          onClick={attachSyncWorkspace}
                        >
                          <Link2 size={16} />
                          {t('绑定当前知识库', 'Attach this library')}
                        </button>
                        <button
                          className="secondary-button"
                          disabled={!canAdministerSync || !!syncBusy}
                          onClick={ensureWorkspaceKey}
                        >
                          <KeyRound size={16} />
                          {t('创建/轮换同步密钥', 'Create/rotate sync key')}
                        </button>
                      </>
                    )}
                  </div>
                )}
                <div className="row-actions">
                  <input
                    className="sync-member-input"
                    value={inviteMemberName}
                    maxLength={80}
                    disabled={!canAdministerSync || !!syncBusy}
                    aria-label={t('成员姓名', 'Member name')}
                    placeholder={t('成员姓名', 'Member name')}
                    onChange={(event) => setInviteMemberName(event.target.value)}
                  />
                  <div className="choice-field sync-role-picker">
                    <span>{t('加入为', 'Join as')}</span>
                    <Select
                      aria-label={t('加入为', 'Join as')}
                      value={inviteRole}
                      disabled={!canAdministerSync || !!syncBusy}
                      onValueChange={(value) => setInviteRole(value as SyncMemberRole)}
                    >
                      <option value="editor">{t('编辑者', 'Editor')}</option>
                      <option value="viewer">{t('只读者', 'Viewer')}</option>
                      <option value="owner">{t('所有者', 'Owner')}</option>
                    </Select>
                  </div>
                  <button
                    className="secondary-button"
                    disabled={!canAdministerSync || !inviteMemberName.trim() || !!syncBusy}
                    onClick={createSyncInvite}
                  >
                    <Link2 size={17} />
                    {t('生成配对码', 'Create pairing code')}
                  </button>
                  <button
                    className="secondary-button"
                    disabled={!canWriteSync || !syncAttached || !!syncBusy}
                    onClick={pushAllNotes}
                  >
                    <ArrowUpFromLine size={17} />
                    {t('推送全部笔记', 'Push all notes')}
                  </button>
                  <button
                    className="secondary-button"
                    disabled={!syncAttached || !!syncBusy}
                    onClick={pullNotes}
                  >
                    <ArrowDownToLine size={17} />
                    {t('拉取并合并', 'Pull and merge')}
                  </button>
                  <button
                    className="secondary-button"
                    disabled={!!syncBusy}
                    onClick={() => void refreshSync()}
                  >
                    <RefreshCw size={17} className={syncBusy === 'refresh' ? 'spin' : undefined} />
                    {t('刷新', 'Refresh')}
                  </button>
                </div>
                {syncInvite && (
                  <div className="sync-invite">
                    <strong>{syncInvite.code}</strong>
                    <span>
                      {t('配对 ID：', 'Invite ID: ')}
                      <code>{syncInvite.invite_id}</code>
                    </span>
                    <span>
                      {syncInvite.member_name ?? t('新设备', 'New device')} ·{' '}
                      {t('角色：', 'Role: ')}
                      {roleLabel(syncInvite.role)} · {t('10 分钟内有效。', 'valid for 10 minutes.')}
                    </span>
                  </div>
                )}
                {syncMembers.length > 0 && (
                  <div className="sync-members">
                    <h3>{t('成员与权限', 'Members and permissions')}</h3>
                    {syncMembers.map((member) => (
                      <div className={`sync-member ${member.status}`} key={member.member_id}>
                        <div>
                          <strong>
                            {member.display_name}
                            {member.member_id === selfSyncDevice?.member_id
                              ? t('（本机成员）', ' (this member)')
                              : ''}
                          </strong>
                          <span>
                            {t(
                              `已连接 ${member.device_count} 台设备`,
                              `${member.device_count} connected devices`,
                            )}
                          </span>
                        </div>
                        <span className={`sync-role-badge ${member.role}`}>
                          {roleLabel(member.role)}
                        </span>
                        <span className={`sync-device-state ${member.status}`}>
                          {member.status === 'active'
                            ? t('活动', 'active')
                            : t('已移除', 'removed')}
                        </span>
                        {canAdministerSync && member.status === 'active' && (
                          <>
                            <div className="choice-field sync-device-role">
                              <span>{t('角色', 'Role')}</span>
                              <Select
                                value={member.role}
                                aria-label={t('成员角色', 'Member role')}
                                disabled={!!syncBusy}
                                onValueChange={(value) =>
                                  changeSyncMemberRole(member.member_id, value as SyncMemberRole)
                                }
                              >
                                <option value="owner">{t('所有者', 'Owner')}</option>
                                <option value="editor">{t('编辑者', 'Editor')}</option>
                                <option value="viewer">{t('只读者', 'Viewer')}</option>
                              </Select>
                            </div>
                            <button
                              className="secondary-button"
                              disabled={!!syncBusy}
                              onClick={() => inviteMemberDevice(member)}
                            >
                              <Link2 size={15} />
                              {t('添加设备', 'Add device')}
                            </button>
                            {member.member_id !== selfSyncDevice?.member_id && (
                              <button
                                className="secondary-button danger-text"
                                disabled={!!syncBusy}
                                onClick={() => setPendingMemberRemoval(member)}
                              >
                                {t('移除成员', 'Remove member')}
                              </button>
                            )}
                          </>
                        )}
                      </div>
                    ))}
                  </div>
                )}{' '}
                {canAdministerSync && syncAudit.length > 0 && (
                  <details className="sync-audit">
                    <summary>
                      <ShieldCheck size={16} />
                      {t('协作审计', 'Collaboration audit')}
                      <span>{t(`最近 ${syncAudit.length} 条`, `latest ${syncAudit.length}`)}</span>
                    </summary>
                    <ol>
                      {syncAudit.map((entry) => (
                        <li key={entry.seq}>
                          <span className={`sync-audit-dot ${entry.action}`} aria-hidden="true" />
                          <div>
                            <strong>{auditActionLabel(entry.action)}</strong>
                            <span>
                              {displayDate(entry.at, language)}
                              {auditDetailText(entry) ? ` · ${auditDetailText(entry)}` : ''}
                            </span>
                          </div>
                        </li>
                      ))}
                    </ol>
                  </details>
                )}
                {syncDevices.length > 0 && (
                  <div className="sync-devices">
                    <h3>{t('已配对设备', 'Paired devices')}</h3>
                    {syncDevices.map((device) => (
                      <div className="sync-device" key={device.device_id}>
                        <div>
                          <strong>
                            {device.display_name}
                            {device.device_id === syncStatus.device_id
                              ? t('（本机）', ' (this device)')
                              : ''}
                          </strong>
                          <span>
                            {device.member_name} · {device.fingerprint}
                          </span>
                          <span className="sync-device-last-seen">
                            {device.last_seen_at
                              ? t(
                                  `最近活动 ${displayDate(device.last_seen_at, language)}`,
                                  `Last active ${displayDate(device.last_seen_at, language)}`,
                                )
                              : t('尚无活动记录', 'No activity yet')}
                          </span>
                        </div>
                        <span className={`sync-role-badge ${device.role}`}>
                          {roleLabel(device.role)}
                        </span>
                        <span className={`sync-device-state ${device.status}`}>
                          {device.status === 'active'
                            ? t('活动', 'active')
                            : t('已撤销', 'revoked')}
                        </span>
                        <span className={`sync-presence ${deviceIsOnline(device) ? 'online' : ''}`}>
                          {deviceIsOnline(device) ? t('在线', 'online') : t('离线', 'offline')}
                        </span>

                        {canAdministerSync &&
                          device.status === 'active' &&
                          device.device_id !== syncStatus.device_id && (
                            <>
                              <button
                                className="secondary-button"
                                disabled={!!syncBusy}
                                onClick={() => grantSyncKey(device.device_id)}
                              >
                                <KeyRound size={15} />
                                {t('授权密钥', 'Grant key')}
                              </button>
                              <button
                                className="secondary-button"
                                disabled={!!syncBusy}
                                onClick={() => revokeSyncDevice(device.device_id)}
                              >
                                {t('撤销', 'Revoke')}
                              </button>
                            </>
                          )}
                      </div>
                    ))}
                  </div>
                )}
                {syncPull && syncPull.server_conflicts.length > 0 && (
                  <div className="sync-conflicts">
                    <h3>{t('需要比较的冲突', 'Conflicts to compare')}</h3>
                    {syncPull.server_conflicts.map((conflict) => (
                      <div className="sync-conflict" key={conflict.target.conflict_id}>
                        <div className="sync-conflict-columns">
                          <div>
                            <strong>{t('本机版本', 'Local')}</strong>
                            <h4>
                              {conflict.local_trashed
                                ? t('（本机已删除）', '(deleted locally)')
                                : conflict.local_title ||
                                  t('（本机无此对象）', '(not on this device)')}
                            </h4>
                            <p>
                              {conflict.local_body ||
                                (conflict.local_trashed
                                  ? t('本机选择删除该对象。', 'This device chose to delete it.')
                                  : t('（无正文）', '(no body)'))}
                            </p>
                          </div>
                          <div>
                            <strong>{t('远端版本', 'Remote')}</strong>
                            <h4>
                              {conflict.target.remote_tombstone
                                ? t('（远端已删除）', '(deleted remotely)')
                                : conflict.remote_title || t('（远端无正文）', '(no remote body)')}
                            </h4>
                            <p>
                              {conflict.remote_body ||
                                (conflict.target.remote_tombstone
                                  ? t(
                                      '远端选择删除该对象。',
                                      'The remote device chose to delete it.',
                                    )
                                  : t('（无正文）', '(no body)'))}
                            </p>
                          </div>
                        </div>
                        <div className="row-actions">
                          <button
                            className="secondary-button"
                            disabled={!!syncBusy}
                            onClick={() => setConflictConfirm({ conflict, mode: 'local' })}
                          >
                            {conflict.local_trashed
                              ? t('保留本机删除', 'Keep local deletion')
                              : conflict.target.remote_tombstone
                                ? t('保留本机并恢复', 'Keep local and restore')
                                : t('保留本机', 'Keep local')}
                          </button>
                          <button
                            className="secondary-button"
                            disabled={!!syncBusy}
                            onClick={() => setConflictConfirm({ conflict, mode: 'remote' })}
                          >
                            {conflict.target.remote_tombstone
                              ? t('采用远端删除', 'Use remote deletion')
                              : t('使用远端', 'Keep remote')}
                          </button>
                          <button
                            className="secondary-button"
                            disabled={
                              !!syncBusy || conflict.local_trashed || !conflict.local_available
                            }
                            onClick={() => setConflictConfirm({ conflict, mode: 'new' })}
                          >
                            {t('另存为新笔记', 'Save as new')}
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
                {pendingConfigConflicts.length > 0 && (
                  <div className="sync-config-conflicts" role="status">
                    <h3>{t('配置冲突', 'Configuration conflicts')}</h3>
                    <p className="backup-small">
                      {t(
                        '本机与远端状态不同，本机内容已保留。请核对具体配置和删除状态，再决定采用哪一方。',
                        'Local and remote states differ. Local content was retained. Review the configuration and any deletion before choosing a version.',
                      )}
                    </p>
                    {pendingConfigConflicts.map((conflict) => (
                      <div
                        className="sync-config-conflict"
                        key={`${conflict.kind}-${conflict.object_id}`}
                      >
                        <div className="sync-config-conflict-copy">
                          <strong>{configKindLabel(conflict.kind, t)}</strong>
                          <span>
                            {conflict.local_tombstone
                              ? t('本机已删除', 'Deleted locally')
                              : conflict.kind === 'workspace_metadata'
                                ? t('本机版本', 'Local version')
                                : conflict.local_title || t('（本机无标题）', '(untitled locally)')}
                          </span>
                          <small>
                            {t('远端：', 'Remote: ')}
                            {conflict.remote_tombstone
                              ? t('远端已删除', 'Deleted remotely')
                              : conflict.kind === 'workspace_metadata'
                                ? t('服务器保留的版本', 'Version kept on the server')
                                : conflict.remote_title || t('（无标题）', '(untitled)')}
                          </small>
                        </div>
                        <div className="row-actions">
                          {(conflict.local_available || conflict.local_tombstone) && (
                            <button
                              className="secondary-button"
                              disabled={!!syncBusy}
                              onClick={() => setConfigConflictConfirm({ conflict, mode: 'local' })}
                            >
                              {t('保留本机并推送', 'Keep and push local')}
                            </button>
                          )}
                          {(conflict.remote_available || conflict.remote_tombstone) && (
                            <button
                              className="secondary-button"
                              disabled={!!syncBusy}
                              onClick={() => setConfigConflictConfirm({ conflict, mode: 'remote' })}
                            >
                              {t('采用远端', 'Use remote')}
                            </button>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                )}{' '}
                {syncPull && syncPull.conflicts.length > 0 && (
                  <p className="backup-small">
                    <Info size={14} />
                    {t(
                      `另有 ${syncPull.conflicts.length} 条本地改动已保留，等待下一次拉取/推送生成可比较记录。`,
                      `${syncPull.conflicts.length} local change(s) are preserved and will become comparable on the next pull/push.`,
                    )}
                  </p>
                )}
                {syncTrash.length > 0 && (
                  <div className="sync-trash">
                    <h3>{t('跨设备回收站', 'Cross-device trash')}</h3>
                    {syncTrash.map((item) => (
                      <div className="sync-trash-item" key={item.object_id}>
                        <span>{item.title || item.object_id}</span>
                        <span>
                          {item.purged
                            ? t('已永久删除', 'Purged')
                            : item.title === null
                              ? t(
                                  '删除于另一设备（本机无正文）',
                                  'Deleted on another device (no body here)',
                                )
                              : t('已删除', 'Deleted')}
                        </span>
                        {!item.purged && item.title !== null && (
                          <button
                            className="secondary-button"
                            disabled={!!syncBusy}
                            onClick={() => restoreSyncTrash(item.object_id)}
                          >
                            <Trash2 size={15} />
                            {t('恢复', 'Restore')}
                          </button>
                        )}
                      </div>
                    ))}
                  </div>
                )}
                {syncMessage && (
                  <p className="backup-small" role="status">
                    <Check size={14} /> {syncMessage}
                  </p>
                )}
                {syncError && syncStatus !== null && (
                  <p className="backup-small sync-error" role="alert">
                    {syncError}
                  </p>
                )}
                {syncBusy && (
                  <p className="backup-small" role="status">
                    <LoaderCircle size={14} className="spin" />
                    {t('同步操作进行中…', 'Sync operation in progress…')}
                  </p>
                )}
              </>
            )}
          </div>
        </section>
        {identity && (
          <TagSyncIssues
            identity={identity}
            refreshVersion={tagRefreshVersion}
            configured={syncStatus?.configured ?? null}
            canPush={
              canWriteSync &&
              syncAttached &&
              !!syncStatus?.healthy &&
              !syncStatus.revoked &&
              !syncBusy
            }
            canReadRemote={
              syncAttached &&
              !!syncStatus?.healthy &&
              !syncStatus.revoked &&
              selfSyncDevice?.status === 'active' &&
              !syncBusy
            }
            readPage={(after, limit) =>
              call(
                { kind: 'sync_tag_page', payload: { after, limit, pending_only: true } },
                'sync_tag_page',
                identity,
              )
            }
            onRetry={(snapshot) =>
              call(
                {
                  kind: 'sync_tag_refresh',
                  payload: { kind: snapshot.kind, entity_id: snapshot.entity_id },
                },
                'sync_tag_snapshot',
                identity,
              )
            }
            onResolve={(snapshot, mode) =>
              call(
                {
                  kind: mode === 'local' ? 'sync_tag_keep_local' : 'sync_tag_keep_remote',
                  payload: {
                    kind: snapshot.kind,
                    entity_id: snapshot.entity_id,
                    remote_version: snapshot.remote_version,
                    local_fingerprint: snapshot.local_fingerprint,
                    local_impact_token: snapshot.local_impact_token,
                  },
                },
                'sync_tag_snapshot',
                identity,
              )
            }
            onOpenItem={onOpenTagSyncItem}
            onOpenTags={onOpenTags}
            onOpenView={onOpenView}
            onChanged={() => {
              if (alive.current) {
                announceTagChange(TAG_METADATA_CHANGED_EVENT, identity);
                announceSyncStatusChanged();
              }
            }}
          />
        )}
        <section className="backup-section">
          <div className="backup-section-icon">
            <HardDrive size={22} />
          </div>
          <div className="backup-section-content">
            <h2>{t('创建完整备份', 'Create a complete backup')}</h2>
            <p>
              {t(
                '保存笔记及历史、资料原件、摘录批注、集合、标签、视图、AI 会话与提案。开始前会先保存当前工作区的修改。',
                'Keep notes and history, source originals, annotations, collections, tags, views, AI conversations and proposals. Current workspace changes are saved before the backup starts.',
              )}
            </p>
            <button
              className="primary-button"
              disabled={!identity || cannotSubmit}
              onClick={() => void createBackup()}
            >
              <ArrowDownToLine size={17} />
              {t('选择位置并备份', 'Choose location and back up')}
            </button>
            {!identity && (
              <p className="backup-small">
                {t(
                  '先打开知识库，即可创建备份。下方的校验与恢复无需开库。',
                  'Open a workspace to create a backup. You can verify and restore a package below without opening one.',
                )}
              </p>
            )}
            <p className="backup-small">
              {t(
                '备份不包含本机未保存草稿、界面设置与临时授权。请使用新的文件名，已有文件不会被覆盖。',
                'Backups exclude unsaved local drafts, interface settings, and temporary permissions. Choose a new filename; existing files are never overwritten.',
              )}
            </p>
          </div>
        </section>
        <section className="backup-section">
          <div className="backup-section-icon">
            <Archive size={22} />
          </div>
          <div className="backup-section-content">
            <h2>{t('检查或恢复备份', 'Check or restore a backup')}</h2>
            <p>
              {t(
                '选择 spanseek 完整备份包，先读取包信息，再进行完整校验或恢复。',
                'Choose a complete spanseek backup to inspect its package details, then verify or restore it.',
              )}
            </p>
            <button
              className="secondary-button"
              disabled={!!activity}
              onClick={() => void selectBackup()}
            >
              <FolderOpen size={17} />
              {selected
                ? t('选择其他备份包', 'Choose another backup')
                : t('选择备份包', 'Choose backup package')}
            </button>
            {activity === 'inspect' && inspectingName && (
              <p className="backup-filename">{inspectingName}</p>
            )}
            {selected && (
              <div className="backup-package">
                <header>
                  <FileArchive size={21} />
                  <div>
                    <strong>{selected.grant.display_name}</strong>
                    <small>{displayDate(selected.inspection.manifest.created_at, language)}</small>
                  </div>
                  <span className="backup-preflight-label">{t('仅预检', 'Preflight only')}</span>
                </header>
                <div className="backup-preflight-note">
                  <Info size={16} />
                  <p>
                    {t(
                      '已读取包结构与清单，尚未完成内容校验。以下数量来自包清单；完整校验将检查文件、笔记历史与引用。',
                      'The package structure and manifest were read. Contents have not been fully verified. Counts below come from the manifest; full verification checks files, revision history, and references.',
                    )}
                  </p>
                </div>
                {selected.inspection.manifest.database_schema >= 14 && (
                  <p>
                    {t(
                      'AI 会话与消息的数量会在完整校验后的报告中显示。',
                      'AI conversation and message counts appear in the report after full verification.',
                    )}
                  </p>
                )}
                <dl className="backup-counts">
                  {(
                    [
                      ['sources', t('资料', 'Sources')],
                      ['source_versions', t('资料版本', 'Source versions')],
                      ['annotations', t('摘录与批注', 'Annotations')],
                      ['web_captures', t('网页快照', 'Web captures')],
                      ['answers', t('依证问答', 'Grounded answers')],
                      ['proposals', t('写作提案', 'Writing proposals')],
                      ['review_entries', t('回顾条目', 'Review entries')],
                    ] as const
                  ).map(([field, label]) => (
                    <div key={field}>
                      <dt>{label}</dt>
                      <dd>{selected.inspection.manifest.counts[field].toLocaleString(language)}</dd>
                    </div>
                  ))}
                  <div>
                    <dt>{t('笔记', 'Notes')}</dt>
                    <dd>{selected.inspection.manifest.counts.notes.toLocaleString(language)}</dd>
                  </div>
                  <div>
                    <dt>{t('历史版本', 'Revisions')}</dt>
                    <dd>
                      {selected.inspection.manifest.counts.revisions.toLocaleString(language)}
                    </dd>
                  </div>
                  <div>
                    <dt>{t('原件与附件文件', 'Original and attachment files')}</dt>
                    <dd>{selected.inspection.manifest.blobs.length.toLocaleString(language)}</dd>
                  </div>
                  <div>
                    <dt>{t('集合', 'Collections')}</dt>
                    <dd>
                      {selected.inspection.manifest.counts.collections.toLocaleString(language)}
                    </dd>
                  </div>
                  <div>
                    <dt>{t('标签', 'Tags')}</dt>
                    <dd>{selected.inspection.manifest.counts.tags.toLocaleString(language)}</dd>
                  </div>
                  <div>
                    <dt>{t('保存的视图', 'Saved views')}</dt>
                    <dd>
                      {selected.inspection.manifest.counts.saved_views.toLocaleString(language)}
                    </dd>
                  </div>
                  <div>
                    <dt>{t('画布', 'Canvases')}</dt>
                    <dd>{selected.inspection.manifest.counts.canvases.toLocaleString(language)}</dd>
                  </div>
                </dl>
                <div className="backup-package-actions">
                  <button
                    className="secondary-button"
                    disabled={cannotSubmit}
                    onClick={() => void useBackup('verify')}
                  >
                    <ShieldCheck size={17} />
                    {t('完整校验', 'Verify fully')}
                  </button>
                  <button
                    className="primary-button"
                    disabled={cannotSubmit}
                    onClick={() => void useBackup('restore')}
                  >
                    <ArrowUpFromLine size={17} />
                    {t('恢复到新文件夹', 'Restore to a new folder')}
                  </button>
                </div>
                <p className="backup-small">
                  {t(
                    '恢复时会重新进行完整校验，只写入新目录，不覆盖当前知识库，也不会混用本机草稿。请在任务成功后打开恢复的新知识库。',
                    'Restore performs full verification again and writes only to a new folder. Your current library and local drafts stay separate. Open the restored workspace after the task succeeds.',
                  )}
                </p>
              </div>
            )}
          </div>
        </section>
      </div>
      {pendingMemberRemoval && (
        <ActionConfirmDialog
          title={t('移除同步成员', 'Remove sync member')}
          description={t(
            `将撤销 ${pendingMemberRemoval.display_name} 的全部设备密钥与访问权限。已有的本地笔记不会被删除，但该成员之后不能继续同步。`,
            `All device keys and access for ${pendingMemberRemoval.display_name} will be revoked. Existing local notes are not deleted, but this member can no longer sync.`,
          )}
          confirmLabel={t('移除成员', 'Remove member')}
          tone="danger"
          onCancel={() => setPendingMemberRemoval(null)}
          onConfirm={() => {
            const member = pendingMemberRemoval;
            setPendingMemberRemoval(null);
            removeSyncMember(member);
          }}
        />
      )}{' '}
      {configConflictConfirm && (
        <ActionConfirmDialog
          title={
            configConflictConfirm.mode === 'local'
              ? t('保留本机配置', 'Keep local configuration')
              : t('采用远端配置', 'Use remote configuration')
          }
          description={configConfirmation(configConflictConfirm, t)}
          confirmLabel={
            configConflictConfirm.mode === 'local'
              ? t('推送本机配置', 'Push local configuration')
              : t('采用远端配置', 'Use remote configuration')
          }
          tone="danger"
          onCancel={() => setConfigConflictConfirm(null)}
          onConfirm={() => {
            const pending = configConflictConfirm;
            setConfigConflictConfirm(null);
            resolveConfigConflict(pending.conflict, pending.mode);
          }}
        />
      )}{' '}
      {conflictConfirm && (
        <SyncConflictConfirmDialog
          mode={conflictConfirm.mode}
          localTitle={conflictConfirm.conflict.local_title ?? ''}
          remoteTitle={conflictConfirm.conflict.remote_title ?? ''}
          localTrashed={conflictConfirm.conflict.local_trashed}
          remoteTombstone={conflictConfirm.conflict.target.remote_tombstone}
          busy={!!syncBusy}
          onCancel={() => setConflictConfirm(null)}
          onConfirm={() => {
            const pending = conflictConfirm;
            setConflictConfirm(null);
            void resolveConflict(pending.conflict.target, pending.mode);
          }}
        />
      )}
    </PageFrame>
  );
}

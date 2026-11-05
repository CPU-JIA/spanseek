import { AlertCircle, Cloud, CloudOff, LoaderCircle, Wifi, WifiOff } from 'lucide-react';
import { useEffect, useState } from 'react';
import type { SyncStatus, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { SYNC_STATUS_CHANGED_EVENT } from './sync-status-events';
import { SYNC_STATUS_UI_TIMEOUT_MS, syncStatusTone } from './sync-status-view';

export function SyncStatusButton({
  identity,
  onOpen,
}: {
  identity: WorkspaceIdentity;
  onOpen: () => void;
}) {
  const { t } = useI18n();
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const key = `${identity.local_instance_id}/${identity.workspace_id}`;

  useEffect(() => {
    let alive = true;
    let inFlight = false;
    async function load() {
      if (inFlight) return;
      inFlight = true;
      try {
        const next = await call(
          { kind: 'sync_status' },
          'sync_status',
          null,
          SYNC_STATUS_UI_TIMEOUT_MS,
        );
        if (!alive) return;
        setStatus(next);
        setError(null);
      } catch (failure) {
        if (!alive) return;
        setError(message(failure));
      } finally {
        inFlight = false;
      }
    }
    void load();
    const timer = window.setInterval(() => void load(), 60_000);
    const refresh = () => void load();
    window.addEventListener(SYNC_STATUS_CHANGED_EVENT, refresh);
    window.addEventListener('focus', refresh);
    return () => {
      alive = false;
      window.clearInterval(timer);
      window.removeEventListener(SYNC_STATUS_CHANGED_EVENT, refresh);
      window.removeEventListener('focus', refresh);
    };
  }, [key]);

  const tone = syncStatusTone(status, identity.workspace_id, error);
  const label =
    tone === 'loading'
      ? t('检查同步…', 'Checking sync…')
      : tone === 'unknown'
        ? t('同步状态未知', 'Sync status unknown')
        : tone === 'unconfigured'
          ? t('同步未开启', 'Sync is off')
          : tone === 'offline'
            ? t('同步离线', 'Sync is offline')
            : tone === 'revoked'
              ? t('同步已撤销', 'Sync access revoked')
              : tone === 'unattached'
                ? t('同步未绑定本库', 'Sync not attached')
                : t('同步已连接', 'Sync connected');
  const detail =
    tone === 'unknown'
      ? error || t('无法读取同步状态。', 'Could not read sync status.')
      : tone === 'unconfigured'
        ? t(
            '尚未配置同步。打开备份与恢复设置。',
            'Sync is not configured. Open Backup & restore to set it up.',
          )
        : tone === 'offline'
          ? t(
              '同步服务器当前不可达；本地编辑仍会保存，恢复连接后再同步。',
              'The sync server is unreachable. Local edits are still saved and can sync after reconnecting.',
            )
          : tone === 'revoked'
            ? t(
                '这台设备已被所有者移除或撤销，不能继续同步。请重新配对。',
                'This device was removed or revoked by an owner and can no longer sync. Pair it again.',
              )
            : tone === 'unattached'
              ? t(
                  '同步已连接，但当前知识库尚未绑定到同步。',
                  'Sync is connected, but this library is not attached to it.',
                )
              : tone === 'online' && status?.endpoint
                ? t(
                    `已连接 ${status.endpoint} · 协议 ${status.protocol_version} · 库 schema ${status.schema_version} · 设备 ${status.device_id?.slice(0, 8) ?? '—'}`,
                    `Connected to ${status.endpoint} · protocol ${status.protocol_version} · library schema ${status.schema_version} · device ${status.device_id?.slice(0, 8) ?? '—'}`,
                  )
                : t('正在检查同步连接。', 'Checking the sync connection.');
  const Icon =
    tone === 'loading'
      ? LoaderCircle
      : tone === 'unknown'
        ? AlertCircle
        : tone === 'unconfigured'
          ? CloudOff
          : tone === 'offline'
            ? WifiOff
            : tone === 'revoked'
              ? AlertCircle
              : tone === 'unattached'
                ? Cloud
                : Wifi;

  return (
    <button
      type="button"
      className={`sync-status-button is-${tone}`}
      aria-label={t(
        `${label}。${detail} 打开备份与恢复。`,
        `${label}. ${detail} Open Backup & restore.`,
      )}
      aria-live="polite"
      title={detail}
      onClick={onOpen}
    >
      <Icon size={16} className={tone === 'loading' ? 'spin' : undefined} aria-hidden="true" />
      <span className="sync-status-button-label">{label}</span>
    </button>
  );
}

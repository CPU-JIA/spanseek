import {
  Archive,
  BookOpen,
  ChevronDown,
  FolderOpen,
  FolderPlus,
  LogOut,
  Settings2,
  Sparkles,
  Activity,
} from 'lucide-react';
import type { DesktopAppState, WorkspaceIdentity } from '@pkms/contracts';
import {
  workspaceAvailabilityLabel,
  workspaceAvailabilityText,
  workspaceIsAvailable,
} from '../welcome/workspace-availability';
import { useI18n } from '../i18n';
import { displayPath } from '../format';
import { PopupMenu, type PopupMenuItem } from '../ui/PopupMenu';
import './workspace-menu.css';

export interface WorkspaceMenuProps {
  app: DesktopAppState | null;
  compact?: boolean;
  busy: boolean;
  onSettings: () => void;
  onModels: () => void;
  onDiagnostics: () => void;
  onBackup: () => void;
  onOpen: () => void;
  onCreate: () => void;
  onClose: () => void;
  onSelect: (identity: WorkspaceIdentity) => void;
}

export function WorkspaceMenu({
  app,
  compact = false,
  busy,
  onSettings,
  onModels,
  onDiagnostics,
  onBackup,
  onOpen,
  onCreate,
  onClose,
  onSelect,
}: WorkspaceMenuProps) {
  const { t } = useI18n();
  const nameOf = (path: string) =>
    path.split(/[\\/]/).filter(Boolean).at(-1) || t('知识库', 'Workspace');
  const name = app?.active ? nameOf(app.active.path) : 'spanseek';
  const active = app?.active?.identity;
  const items: PopupMenuItem[] = [
    { id: 'settings', label: t('设置', 'Settings'), icon: <Settings2 />, onSelect: onSettings },
    {
      id: 'models',
      label: t('模型与检索', 'Models & search'),
      icon: <Sparkles />,
      onSelect: onModels,
      disabled: !active,
    },
    {
      id: 'backup',
      label: t('备份与恢复', 'Backup & restore'),
      icon: <Archive />,
      onSelect: onBackup,
    },
    {
      id: 'diagnostics',
      label: t('统计与诊断', 'Statistics & diagnostics'),
      icon: <Activity />,
      onSelect: onDiagnostics,
      disabled: !active,
    },
    {
      id: 'open',
      label: t('打开知识库…', 'Open workspace…'),
      icon: <FolderOpen />,
      separatorBefore: true,
      onSelect: onOpen,
    },
    {
      id: 'create',
      label: t('新建知识库…', 'New workspace…'),
      icon: <FolderPlus />,
      onSelect: onCreate,
    },
    ...(app?.workspaces ?? []).map((entry, index): PopupMenuItem => {
      const current =
        entry.identity.workspace_id === active?.workspace_id &&
        entry.identity.local_instance_id === active?.local_instance_id;
      const available = workspaceIsAvailable(entry.availability);
      const reason = workspaceAvailabilityText(entry.availability, t);
      return {
        id: `workspace-${entry.identity.local_instance_id}-${entry.identity.workspace_id}`,
        label: `${nameOf(entry.path)} · ${displayPath(entry.path)}${
          available ? '' : `（${workspaceAvailabilityLabel(entry.availability, t)}）`
        }`,
        detail: reason ?? undefined,
        icon: <BookOpen />,
        checked: current,
        radio: true,
        disabled: !available,
        separatorBefore: index === 0,
        groupLabel: index === 0 ? t('最近的知识库', 'Recent workspaces') : undefined,
        onSelect: () => {
          if (!current) onSelect(entry.identity);
        },
      };
    }),
    ...(app?.active
      ? [
          {
            id: 'close',
            label: t('关闭知识库', 'Close workspace'),
            icon: <LogOut />,
            separatorBefore: true,
            onSelect: onClose,
          },
        ]
      : []),
  ];
  return (
    <PopupMenu
      label={t(`知识库菜单：${name}`, `Workspace menu: ${name}`)}
      className={`workspace-button workspace-menu-trigger${compact ? ' workspace-menu-compact' : ''}`}
      menuClassName="workspace-menu"
      placement="bottom-start"
      disabled={busy}
      items={items}
      trigger={
        <>
          <span className="brand-mark" aria-hidden="true">
            <BookOpen size={18} />
          </span>
          <span className={compact ? 'sr-only' : undefined}>
            <strong>{name}</strong>
            <small>{t('个人知识库', 'Personal workspace')}</small>
          </span>
          {!compact && <ChevronDown size={14} aria-hidden="true" />}
        </>
      }
    />
  );
}

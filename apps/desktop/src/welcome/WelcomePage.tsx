import { useId, useRef, useState } from 'react';
import type { WorkspaceEntry, WorkspaceIdentity } from '@pkms/contracts';
import {
  Activity,
  Archive,
  FolderOpen,
  FolderPlus,
  FileUp,
  RefreshCw,
  Search,
  Settings2,
  X,
} from 'lucide-react';
import { displayDate, displayPath } from '../format';
import {
  workspaceAvailabilityLabel,
  workspaceAvailabilityText,
  workspaceIsAvailable,
} from './workspace-availability';
import { useI18n } from '../i18n';
import { Tooltip } from '../ui/Tooltip';
import { BrandMark } from '../ui/BrandMark';
import './welcome.css';

type WelcomePageProps = {
  workspaces: readonly WorkspaceEntry[];
  busy: boolean;
  desktopAvailable: boolean;
  /** True when the local service cannot be reached at all, so retrying helps. */
  backendUnavailable: boolean;
  onActivity: () => void;
  onSettings: () => void;
  onCreate: () => void;
  onOpen: (identity?: WorkspaceIdentity) => void;
  onForget: (identity: WorkspaceIdentity) => void;
  onRestore: () => void;
  onReconnect: () => void;
  onOpenPdf?: () => void;
};

function searchablePath(path: string) {
  return displayPath(path).replaceAll('\\', '/').toLocaleLowerCase();
}

export function WelcomePage({
  workspaces,
  busy,
  desktopAvailable,
  backendUnavailable,
  onActivity,
  onSettings,
  onCreate,
  onOpen,
  onForget,
  onRestore,
  onReconnect,
  onOpenPdf,
}: WelcomePageProps) {
  const { t, language } = useI18n();
  const id = useId();
  const [query, setQuery] = useState('');
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const normalizedQuery = searchablePath(query.trim());
  const visible = normalizedQuery
    ? workspaces.filter((entry) => searchablePath(entry.path).includes(normalizedQuery))
    : workspaces;

  function updateQuery(value: string) {
    setQuery(value);
    if (list.current) list.current.scrollTop = 0;
  }

  function clearQuery() {
    updateQuery('');
    search.current?.focus();
  }

  return (
    <section className="welcome">
      <header className="welcome-header">
        <div className="welcome-brand">
          <BrandMark size={22} />
          <span className="welcome-brand-name">
            <strong>跨索</strong>
            <small>spanseek</small>
          </span>
        </div>
        <div className="welcome-tools">
          <Tooltip label={t('任务中心', 'Activity')}>
            <button
              type="button"
              className="icon-button"
              aria-label={t('任务中心', 'Activity')}
              onClick={onActivity}
            >
              <Activity size={18} aria-hidden="true" />
            </button>
          </Tooltip>
          <Tooltip label={t('设置', 'Settings')} shortcut="Ctrl + ,">
            <button
              type="button"
              className="icon-button"
              aria-label={t('设置', 'Settings')}
              onClick={onSettings}
            >
              <Settings2 size={18} aria-hidden="true" />
            </button>
          </Tooltip>
        </div>
      </header>
      <div className="welcome-layout">
        <div className="welcome-intro">
          <p className="eyebrow">YOUR KNOWLEDGE, CONNECTED.</p>
          <h1>{t('给思考一个生长的地方。', 'A home for your thoughts.')}</h1>
          <p className="welcome-copy">
            {t('随手记录，慢慢连接。', 'Capture a thought. Make a connection.')}
            <br />
            {t('从一个属于自己的本地知识库开始。', 'Start with a local workspace of your own.')}
          </p>
          <div className="welcome-actions">
            <button
              type="button"
              className="primary-button"
              disabled={busy || !desktopAvailable}
              onClick={onCreate}
            >
              <FolderPlus size={18} aria-hidden="true" />
              {t('创建知识库', 'Create workspace')}
            </button>
            <button
              type="button"
              className="secondary-button"
              disabled={busy || !desktopAvailable}
              onClick={() => onOpen()}
            >
              <FolderOpen size={18} aria-hidden="true" />
              {t('打开已有知识库', 'Open a workspace')}
            </button>
          </div>
          {onOpenPdf && (
            <div className="welcome-open-document">
              <button
                type="button"
                className="secondary-button"
                disabled={busy || !desktopAvailable}
                onClick={onOpenPdf}
              >
                <FileUp size={18} aria-hidden="true" />
                {t('打开 PDF', 'Open PDF')}
                <kbd>Ctrl + O</kbd>
              </button>
              <p>
                {t(
                  '无需先创建知识库。阅读后可随时保存进库。',
                  'Start reading without a workspace. Save it to a workspace when ready.',
                )}
              </p>
            </div>
          )}
          <button type="button" className="text-button" onClick={onRestore}>
            <Archive size={15} aria-hidden="true" />
            {t('从完整备份恢复', 'Restore from a full backup')}
          </button>
          {backendUnavailable && (
            <button type="button" className="secondary-button" onClick={onReconnect}>
              <RefreshCw size={17} aria-hidden="true" />
              {t('重新连接', 'Reconnect')}
            </button>
          )}
        </div>
        <section
          className="recent-workspaces"
          aria-labelledby={`${id}-recent-title`}
          aria-busy={busy}
        >
          <div className="recent-workspaces-heading">
            <h2 id={`${id}-recent-title`}>{t('最近的知识库', 'Recent workspaces')}</h2>
            {workspaces.length > 0 && (
              <span
                className="recent-workspaces-count"
                role="status"
                aria-label={t(
                  `显示 ${visible.length} 个，共 ${workspaces.length} 个知识库`,
                  `${visible.length} of ${workspaces.length} workspaces`,
                )}
              >
                {normalizedQuery ? `${visible.length} / ${workspaces.length}` : workspaces.length}
              </span>
            )}
          </div>
          {workspaces.length > 0 && (
            <div className="recent-workspaces-search">
              <Search size={16} aria-hidden="true" />
              <input
                ref={search}
                type="search"
                value={query}
                aria-label={t('搜索最近的知识库', 'Search recent workspaces')}
                placeholder={t('搜索名称或路径', 'Search name or path')}
                onChange={(event) => updateQuery(event.target.value)}
              />
              {query && (
                <button
                  type="button"
                  className="recent-workspaces-clear"
                  aria-label={t('清除搜索', 'Clear search')}
                  onClick={clearQuery}
                >
                  <X size={14} aria-hidden="true" />
                </button>
              )}
            </div>
          )}
          <div className="recent-workspaces-content">
            {visible.length > 0 ? (
              <ul
                ref={list}
                className="recent-workspace-list"
                aria-label={t('最近知识库列表', 'Recent workspace list')}
                tabIndex={0}
              >
                {visible.map((entry) => {
                  const path = displayPath(entry.path);
                  const name =
                    path.split(/[\\/]/).filter(Boolean).at(-1) || t('知识库', 'Workspace');
                  const available = workspaceIsAvailable(entry.availability);
                  const reason = workspaceAvailabilityText(entry.availability, t);
                  return (
                    <li key={entry.identity.local_instance_id}>
                      <div className="recent-workspace-row">
                        <button
                          type="button"
                          className="recent-workspace"
                          title={reason ? `${path} — ${reason}` : path}
                          disabled={!available || busy}
                          onClick={() => onOpen(entry.identity)}
                        >
                          <FolderOpen size={18} strokeWidth={1.5} aria-hidden="true" />
                          <span className="recent-workspace-details">
                            <strong>{name}</strong>
                            <small>{path}</small>
                            <small className="recent-workspace-opened">
                              {t('最后打开：', 'Last opened: ') +
                                displayDate(entry.last_opened_at, language)}
                            </small>
                            {reason && <small className="recent-workspace-reason">{reason}</small>}
                          </span>
                          <span className="recent-workspace-action">
                            {workspaceAvailabilityLabel(entry.availability, t)}
                          </span>
                        </button>
                        <Tooltip
                          label={t(
                            '从最近列表移除（不删除知识库）',
                            'Remove from recent list (keeps the workspace)',
                          )}
                        >
                          <button
                            type="button"
                            className="recent-workspace-forget"
                            aria-label={t('从最近列表移除：', 'Remove from recent list: ') + name}
                            disabled={busy}
                            onClick={() => onForget(entry.identity)}
                          >
                            <X size={14} aria-hidden="true" />
                          </button>
                        </Tooltip>
                      </div>
                    </li>
                  );
                })}
              </ul>
            ) : (
              <div className="welcome-empty">
                {workspaces.length > 0 ? (
                  <Search size={26} aria-hidden="true" />
                ) : (
                  <FolderOpen size={28} aria-hidden="true" />
                )}
                <h3>
                  {workspaces.length > 0
                    ? t('未找到匹配的知识库', 'No matching workspaces')
                    : t('还没有最近的知识库', 'No recent workspaces yet')}
                </h3>
                <p>
                  {workspaces.length > 0
                    ? t(
                        '试试知识库名称或路径中的关键词。',
                        'Try a workspace name or part of its path.',
                      )
                    : t(
                        '创建或打开一个知识库，下次就能在这里直接进入。',
                        'Create or open a workspace to find it here next time.',
                      )}
                </p>
                {workspaces.length > 0 && (
                  <button type="button" className="text-button" onClick={clearQuery}>
                    {t('清除搜索', 'Clear search')}
                  </button>
                )}
              </div>
            )}
          </div>
        </section>
      </div>
    </section>
  );
}

import { useId, useLayoutEffect, useRef, type ReactNode } from 'react';
import type { OpenTab, PaneState } from '@pkms/contracts';
import { BookOpen, FileText, Layers, MoreHorizontal, Plus, Sparkles, X } from 'lucide-react';
import { useI18n } from '../i18n';
import { PopupMenu } from '../ui/PopupMenu';
import { Tooltip } from '../ui/Tooltip';
import type { TabCloseMode } from './tab-closing';
import './workspace-tabs.css';

export type WorkspaceTabsProps = {
  panes: PaneState[];
  activePane: number;
  contentActive: boolean;
  busy: boolean;
  renderTitle: (tab: OpenTab) => ReactNode;
  onOpen: (paneIndex: number, tab: OpenTab) => void;
  onClose: (paneIndex: number, objectId: string, mode: TabCloseMode) => void;
  onNew: () => void;
};

const tabIcons = { note: FileText, source: BookOpen, topic: Layers, answer: Sparkles };

export function WorkspaceTabs({
  panes,
  activePane,
  contentActive,
  busy,
  renderTitle,
  onOpen,
  onClose,
  onNew,
}: WorkspaceTabsProps) {
  const { t } = useI18n();
  const id = useId();
  const list = useRef<HTMLDivElement>(null);
  const activeRow = useRef<HTMLLIElement>(null);
  const activeObjectId = panes[activePane]?.active_object_id;
  const count = panes.reduce((total, pane) => total + pane.tabs.length, 0);

  useLayoutEffect(() => {
    const viewport = list.current;
    if (!viewport) return;
    const revealActive = () => {
      const row = activeRow.current;
      if (!row || !viewport.clientHeight) return;
      const bounds = viewport.getBoundingClientRect();
      const item = row.getBoundingClientRect();
      if (item.top < bounds.top) viewport.scrollTop += item.top - bounds.top;
      else if (item.bottom > bounds.bottom) viewport.scrollTop += item.bottom - bounds.bottom;
    };
    // Scroll only this list, so opening a document cannot shift the main navigation.
    revealActive();
    const observer = new ResizeObserver(revealActive);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [activePane, activeObjectId, contentActive]);

  return (
    <section className="workspace-tabs" aria-labelledby={`${id}-heading`} aria-busy={busy}>
      <div className="workspace-tabs-heading">
        <h2 id={`${id}-heading`}>{t('标签页', 'Tabs')}</h2>
        {count > 0 ? <span className="workspace-tabs-count">{count}</span> : null}
      </div>
      <button type="button" className="workspace-tabs-new" disabled={busy} onClick={onNew}>
        <Plus size={16} strokeWidth={1.7} aria-hidden="true" />
        <span>{t('新建笔记', 'New note')}</span>
      </button>
      <span id={`${id}-close-label`} className="sr-only">
        {t('关闭标签页', 'Close tab')}
      </span>
      <div className="workspace-tabs-list" ref={list}>
        {panes.length === 0 ? (
          <p className="workspace-tabs-empty">{t('尚未打开文档', 'No open documents')}</p>
        ) : (
          panes.map((pane, paneIndex) => (
            <div
              className="workspace-tabs-group"
              key={paneIndex}
              role={panes.length > 1 ? 'group' : undefined}
              aria-labelledby={panes.length > 1 ? `${id}-pane-${paneIndex}` : undefined}
            >
              {panes.length > 1 ? (
                <div className="workspace-tabs-group-heading" id={`${id}-pane-${paneIndex}`}>
                  <span>{t(`面板 ${paneIndex + 1}`, `Pane ${paneIndex + 1}`)}</span>
                  {contentActive && paneIndex === activePane ? (
                    <span className="workspace-tabs-current-pane">{t('当前', 'Current')}</span>
                  ) : null}
                </div>
              ) : null}
              {pane.tabs.length === 0 ? (
                <p className="workspace-tabs-empty">{t('尚未打开文档', 'No open documents')}</p>
              ) : (
                <ul>
                  {pane.tabs.map((tab) => {
                    const latest = paneIndex === activePane && tab.object_id === activeObjectId;
                    const selected = contentActive && latest;
                    const titleId = `${id}-title-${paneIndex}-${tab.object_id}`;
                    const Icon = tabIcons[tab.kind];
                    return (
                      <li
                        key={`${tab.kind}:${tab.object_id}`}
                        ref={latest ? activeRow : undefined}
                        className={`workspace-tab${selected ? ' active' : ''}`}
                      >
                        <button
                          type="button"
                          className="workspace-tab-open"
                          disabled={busy}
                          data-workspace-tab-open=""
                          data-pane-index={paneIndex}
                          aria-current={selected ? 'page' : undefined}
                          onClick={() => onOpen(paneIndex, tab)}
                        >
                          <Icon size={16} strokeWidth={1.7} aria-hidden="true" />
                          <span id={titleId} className="workspace-tab-title">
                            {renderTitle(tab)}
                          </span>
                        </button>
                        <div className="workspace-tab-actions">
                          <PopupMenu
                            label={t('标签页操作', 'Tab actions')}
                            className="workspace-tab-more"
                            trigger={<MoreHorizontal size={15} strokeWidth={1.7} />}
                            disabled={busy}
                            items={[
                              {
                                id: 'close',
                                label: t('关闭此标签', 'Close this tab'),
                                shortcut: selected ? 'Ctrl+W' : undefined,
                                icon: <X size={15} />,
                                onSelect: () => onClose(paneIndex, tab.object_id, 'this'),
                              },
                              {
                                id: 'close-others',
                                label: t('关闭同面板其他标签', 'Close other tabs in this pane'),
                                disabled: pane.tabs.length < 2,
                                onSelect: () => onClose(paneIndex, tab.object_id, 'others'),
                              },
                              {
                                id: 'close-all',
                                label: t('关闭同面板全部标签', 'Close all tabs in this pane'),
                                onSelect: () => onClose(paneIndex, tab.object_id, 'all'),
                              },
                            ]}
                          />
                          <Tooltip label={t('关闭标签页', 'Close tab')} shortcut="Ctrl+W">
                            <button
                              type="button"
                              className="workspace-tab-close"
                              disabled={busy}
                              aria-labelledby={`${id}-close-label ${titleId}`}
                              onClick={() => onClose(paneIndex, tab.object_id, 'this')}
                            >
                              <X size={13} strokeWidth={1.7} aria-hidden="true" />
                            </button>
                          </Tooltip>
                        </div>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          ))
        )}
      </div>
    </section>
  );
}

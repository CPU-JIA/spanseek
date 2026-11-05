import {
  BookOpen,
  ChevronDown,
  FolderClosed,
  Inbox,
  LayoutGrid,
  LibraryBig,
  ListFilter,
  Network,
  RefreshCw,
  Sparkles,
  Tags,
} from 'lucide-react';
import { useI18n } from '../i18n';
import type { ViewName } from './navigation-view';
import type { ProductSection } from './product-navigation';
import { usePageNavigationState } from './page-navigation-state';
import './product-navigation.css';

type Props = {
  view: ViewName;
  section: ProductSection;
  ready: boolean;
  onNavigate: (view: ViewName) => void;
  onKnowledge: (mode: 'all' | 'inbox') => void;
};

export function ProductNavigation({ view, section, ready, onNavigate, onKnowledge }: Props) {
  const { t } = useI18n();
  const [expandedGroups, setExpandedGroups] = usePageNavigationState<Record<string, boolean>>(
    'sidebar.expanded-groups',
    {},
  );
  const entries = [
    {
      id: 'knowledge',
      label: t('知识索引', 'Knowledge index'),
      icon: LibraryBig,
      action: () => onKnowledge('all'),
    },
    { id: 'inbox', label: t('收件箱', 'Inbox'), icon: Inbox, action: () => onKnowledge('inbox') },
    {
      id: 'sources',
      label: t('资料库', 'Library'),
      icon: BookOpen,
      action: () => onNavigate('sources'),
    },
    {
      id: 'graph',
      label: t('知识图谱', 'Knowledge graph'),
      icon: Network,
      action: () => onNavigate('graph'),
    },
    {
      id: 'review',
      label: t('回顾', 'Review'),
      icon: RefreshCw,
      action: () => onNavigate('review'),
    },
    {
      id: 'answers',
      label: t('AI 助手', 'AI assistant'),
      icon: Sparkles,
      action: () => onNavigate('answers'),
    },
  ] as const;
  const children: Partial<
    Record<ProductSection, { view: ViewName; label: string; icon: typeof Tags }[]>
  > = {
    knowledge: [
      { view: 'collections', label: t('管理分类', 'Manage categories'), icon: FolderClosed },
      { view: 'tags', label: t('标签管理', 'Tags'), icon: Tags },
      { view: 'views', label: t('保存视图', 'Saved views'), icon: ListFilter },
    ],
    graph: [{ view: 'canvas', label: t('画布', 'Canvases'), icon: LayoutGrid }],
    answers: [{ view: 'applications', label: t('知识任务', 'Knowledge tasks'), icon: Sparkles }],
  };
  return (
    <nav
      className="workspace-navigation product-navigation"
      aria-label={t('主导航', 'Main navigation')}
    >
      <div className="nav-primary">
        {entries.map(({ id, label, icon: Icon, action }) => {
          const nested = children[id] ?? [];
          const childActive = nested.some((entry) => entry.view === view);
          const expanded = expandedGroups[id] ?? section === id;
          return (
            <div className="product-nav-group" key={id}>
              <div className="product-nav-heading">
                <button
                  className={`nav-item product-nav-${id} ${section === id ? 'active' : ''}`}
                  aria-current={section === id && (!childActive || !expanded) ? 'page' : undefined}
                  disabled={!ready}
                  onClick={action}
                >
                  <Icon className="nav-icon" aria-hidden="true" />
                  <span>{label}</span>
                </button>
                {nested.length > 0 && (
                  <button
                    type="button"
                    className="product-nav-toggle"
                    disabled={!ready}
                    aria-expanded={expanded}
                    aria-controls={`product-nav-${id}-children`}
                    aria-label={
                      expanded
                        ? t(`收起${label}子导航`, `Collapse ${label} navigation`)
                        : t(`展开${label}子导航`, `Expand ${label} navigation`)
                    }
                    onClick={() =>
                      setExpandedGroups((current) => ({ ...current, [id]: !expanded }))
                    }
                  >
                    <ChevronDown size={14} aria-hidden="true" />
                  </button>
                )}
              </div>
              {ready && expanded && nested.length > 0 && (
                <div className="product-nav-children" id={`product-nav-${id}-children`}>
                  {nested.map(({ view: target, label: childLabel, icon: ChildIcon }) => (
                    <button
                      key={target}
                      className={view === target ? 'is-current' : ''}
                      aria-current={view === target ? 'page' : undefined}
                      onClick={() => onNavigate(target)}
                    >
                      <ChildIcon size={15} aria-hidden="true" />
                      <span>{childLabel}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </nav>
  );
}

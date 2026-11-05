import type { Navigation } from '@pkms/contracts';

/** Every page the workspace can show. */
export type ViewName =
  | 'knowledge'
  | 'applications'
  | 'settings'
  | 'tags'
  | 'notes'
  | 'voice'
  | 'graph'
  | 'operations'
  | 'collections'
  | 'views'
  | 'canvas'
  | 'backup'
  | 'trash'
  | 'sources'
  | 'reader'
  | 'search'
  | 'models'
  | 'answers'
  | 'proposal'
  | 'review'
  | 'automation'
  | 'diagnostics';

export type NavigationRestore = {
  view: ViewName;
  /** Legacy callers may still redirect inbox into the old notes workbench. */
  inbox: 'pending' | null;
  /** Page-scoped state that must be restored with the screen key. */
  pageState?: Record<string, unknown>;
};

/**
 * The persisted sidebar entry a page belongs to.
 *
 * The generated app-state contract still uses its historical navigation
 * vocabulary. Active styling must use the real ViewName; this mapping only
 * controls what is safe to restore after a restart.
 */
export function navigationForView(view: ViewName): Navigation {
  switch (view) {
    case 'applications':
      return 'applications';
    case 'settings':
      return 'settings';
    case 'knowledge':
      return 'knowledge';
    case 'voice':
      // Voice capture is a secondary workspace surface; restoring it after a
      // restart can leave the workbench without a note, so the index is safer.
      return 'knowledge';
    case 'sources':
    case 'reader':
      return 'sources';
    case 'graph':
      return 'graph';
    case 'review':
    case 'automation':
      return 'review';
    case 'operations':
      return 'tasks';
    case 'trash':
      return 'trash';
    case 'collections':
    case 'views':
    case 'canvas':
      return 'topics';
    // The remaining detail/context/maintenance pages do not own a primary
    // navigation item. Returning to the knowledge index is safer than restoring
    // an unrelated legacy page such as the notes list.
    default:
      return 'knowledge';
  }
}

/**
 * The page a stored sidebar entry should restore after a restart.
 *
 * `inbox` is not a second page: it is the knowledge index in inbox mode.
 */
export function viewForNavigation(navigation: Navigation): NavigationRestore {
  switch (navigation) {
    case 'applications':
      return { view: 'applications', inbox: null };
    case 'settings':
      return { view: 'settings', inbox: null };
    case 'knowledge':
      return { view: 'knowledge', inbox: null };
    case 'inbox':
      return {
        view: 'knowledge',
        inbox: null,
        pageState: { 'knowledge.mode': 'inbox' },
      };
    case 'graph':
      return { view: 'graph', inbox: null };
    case 'sources':
      return { view: 'sources', inbox: null };
    case 'review':
      return { view: 'review', inbox: null };
    case 'tasks':
      return { view: 'operations', inbox: null };
    case 'trash':
      return { view: 'trash', inbox: null };
    case 'topics':
      return { view: 'collections', inbox: null };
    case 'notes':
      return { view: 'knowledge', inbox: null };
    // Historical search/notes entries no longer own a primary screen.
    default:
      return { view: 'knowledge', inbox: null };
  }
}

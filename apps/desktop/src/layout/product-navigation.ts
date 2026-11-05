import type { ViewName } from './navigation-view.ts';

export type ProductSection =
  'knowledge' | 'inbox' | 'sources' | 'graph' | 'review' | 'answers' | 'tools';

/** Content keeps its entry context without adding a navigation level. */
export function productSection(
  view: ViewName,
  inbox = false,
  contentOrigin?: ProductSection,
): ProductSection {
  switch (view) {
    case 'notes':
      return contentOrigin ?? 'knowledge';
    case 'reader':
      return contentOrigin ?? 'sources';
    case 'knowledge':
      return inbox ? 'inbox' : 'knowledge';
    case 'sources':
      return 'sources';
    case 'graph':
    case 'canvas':
      return 'graph';
    case 'review':
      return 'review';
    case 'applications':
    case 'answers':
    case 'proposal':
      return 'answers';
    case 'settings':
    case 'models':
    case 'diagnostics':
    case 'backup':
    case 'automation':
    case 'operations':
    case 'trash':
    case 'voice':
      return 'tools';
    default:
      return 'knowledge';
  }
}

import type { ChangeSetSummary, ResearchTaskSummary, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { readBoundedCursorPage } from './bounded-cursor-page';

// Core caps both endpoints at 50; the UI also offers 100 rows per page.
export function readResearchPage(identity: WorkspaceIdentity, cursor: string | null, size: number) {
  return readBoundedCursorPage<ResearchTaskSummary>(cursor, size, 50, async (before_id, limit) => {
    const page = await call(
      { kind: 'research_page', payload: { before_id, limit } },
      'research_page',
      identity,
    );
    return { items: page.items, next_cursor: page.next_before_id };
  });
}
export function readChangeSetPage(
  identity: WorkspaceIdentity,
  cursor: string | null,
  size: number,
) {
  return readBoundedCursorPage<ChangeSetSummary>(cursor, size, 50, async (before_id, limit) => {
    const page = await call(
      {
        kind: 'automation_change_sets_page',
        payload: { before_id, limit, kind: null, status: null },
      },
      'change_set_page',
      identity,
    );
    return { items: page.items, next_cursor: page.next_before_id };
  });
}

import type { SessionLayout } from '@pkms/contracts';

/** Remove a recycled note from both panes without dropping unrelated tabs or sizing. */
export function withoutNote(layout: SessionLayout, noteId: string): SessionLayout {
  return withoutObjects(layout, [noteId]);
}

/** A confirmed deletion invalidates every open occurrence, including source/topic tabs. */
export function withoutObjects(layout: SessionLayout, objectIds: readonly string[]): SessionLayout {
  const removed = new Set(objectIds);
  return {
    ...layout,
    panes: layout.panes.map((pane) => {
      const tabs = pane.tabs.filter((tab) => !removed.has(tab.object_id));
      const activeIndex = pane.tabs.findIndex((tab) => tab.object_id === pane.active_object_id);
      const following = pane.tabs.slice(activeIndex + 1).find((tab) => !removed.has(tab.object_id));
      const preceding = pane.tabs
        .slice(0, activeIndex)
        .reverse()
        .find((tab) => !removed.has(tab.object_id));
      return {
        ...pane,
        tabs,
        active_object_id:
          pane.active_object_id && removed.has(pane.active_object_id)
            ? (following?.object_id ?? preceding?.object_id ?? null)
            : pane.active_object_id,
        scroll_offsets: pane.scroll_offsets.filter((entry) => !removed.has(entry.object_id)),
      };
    }),
  };
}

/** Keep the next visible item, or its predecessor, after a selection leaves Trash. */
export function nextTrashFocus(
  visibleIds: readonly string[],
  removedIds: readonly string[],
  activeId: string | null,
): string | null {
  const removed = new Set(removedIds);
  if (activeId && visibleIds.includes(activeId) && !removed.has(activeId)) return activeId;
  const index = activeId ? visibleIds.indexOf(activeId) : -1;
  return (
    visibleIds.slice(index + 1).find((id) => !removed.has(id)) ??
    visibleIds
      .slice(0, Math.max(index, 0))
      .reverse()
      .find((id) => !removed.has(id)) ??
    null
  );
}

import type { SessionLayout } from '@pkms/contracts';
import { navigateAfterBarriers } from './navigation-guard.ts';
import { modalGuard } from './modal-guard.ts';

export type TabCloseMode = 'this' | 'others' | 'all';
export type TabClosePlan = {
  paneIndex: number;
  objectIds: string[];
  keepId: string | null;
  position: number;
};

/** Freeze the affected tabs before saving; newly opened tabs never join a pending close. */
export function planTabClose(
  layout: SessionLayout,
  paneIndex: number,
  objectId: string,
  mode: TabCloseMode,
): TabClosePlan | null {
  const pane = layout.panes[paneIndex];
  const position = pane?.tabs.findIndex((tab) => tab.object_id === objectId) ?? -1;
  if (!pane || position < 0) return null;
  const objectIds = pane.tabs
    .filter(
      (tab) =>
        mode === 'all' ||
        (mode === 'this' ? tab.object_id === objectId : tab.object_id !== objectId),
    )
    .map((tab) => tab.object_id);
  return objectIds.length
    ? { paneIndex, objectIds, keepId: mode === 'others' ? objectId : null, position }
    : null;
}

/** One idempotent layout target, including active-document and reading-position cleanup. */
export function applyTabClose(layout: SessionLayout, plan: TabClosePlan): SessionLayout {
  const pane = layout.panes[plan.paneIndex];
  if (!pane || (plan.keepId && !pane.tabs.some((tab) => tab.object_id === plan.keepId))) {
    return layout;
  }
  const ids = new Set(plan.objectIds);
  const tabs = pane.tabs.filter((tab) => !ids.has(tab.object_id));
  const scrollOffsets = pane.scroll_offsets.filter((offset) => !ids.has(offset.object_id));
  if (tabs.length === pane.tabs.length && scrollOffsets.length === pane.scroll_offsets.length) {
    return layout;
  }
  const next = {
    ...pane,
    tabs,
    scroll_offsets: scrollOffsets,
    active_object_id:
      pane.active_object_id && ids.has(pane.active_object_id)
        ? (tabs.at(-1)?.object_id ?? null)
        : pane.active_object_id,
  };
  return {
    ...layout,
    panes: layout.panes.map((item, index) => (index === plan.paneIndex ? next : item)),
  };
}

export function closeTabsAfterBarriers(
  plan: TabClosePlan,
  barriers: Array<() => Promise<unknown>>,
  current: () => boolean,
  updateLayout: (change: (layout: SessionLayout) => SessionLayout) => Promise<unknown>,
): Promise<boolean> {
  return navigateAfterBarriers(barriers, current, () =>
    updateLayout((layout) => (current() ? applyTabClose(layout, plan) : layout)),
  );
}

type CloseKey = Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'>;

/** Match the chord separately so an empty view can still suppress native window closing. */
export function isTabCloseKey(event: CloseKey): boolean {
  return (
    event.key.toLowerCase() === 'w' &&
    (event.ctrlKey || event.metaKey) &&
    !event.shiftKey &&
    !event.altKey
  );
}

type CloseShortcutEvent = CloseKey &
  Pick<KeyboardEvent, 'defaultPrevented' | 'preventDefault' | 'isComposing' | 'keyCode' | 'repeat'>;

export function handleTabCloseKey(
  event: CloseShortcutEvent,
  context: {
    layout: SessionLayout | null;
    contentActive: boolean;
    blocked: boolean;
    close: (paneIndex: number, objectId: string) => void;
  },
): boolean {
  if (!isTabCloseKey(event)) return false;
  if (event.defaultPrevented) return true;
  event.preventDefault();
  if (
    context.blocked ||
    !context.contentActive ||
    event.isComposing ||
    event.keyCode === 229 ||
    event.repeat
  ) {
    return true;
  }
  const layout = context.layout;
  const pane = layout?.panes[layout.active_pane];
  const objectId = pane?.active_object_id;
  if (layout && objectId && pane?.tabs.some((tab) => tab.object_id === objectId)) {
    context.close(layout.active_pane, objectId);
  }
  return true;
}

/** Called after the confirmed layout renders, for buttons, menus and the keyboard alike. */
export function restoreTabCloseFocus(plan: TabClosePlan, previous: Element | null): void {
  const doc = previous?.ownerDocument ?? document;
  const active = doc.activeElement;
  if (active !== doc.body && active !== previous) return;
  if (previous && previous !== doc.body && previous.isConnected && active === previous) return;
  // Native `<dialog>.showModal()` is the modal of this app: it is what `dialog[open]`
  // matches. The ARIA alternative stays as the contract for a future non-native
  // modal — `role="dialog"` alone is not modal, which is why the tag filter panel
  // (sources/SourceFilters.tsx) is not counted here either.
  const modal = doc.querySelector(modalGuard);
  const usable = (element: HTMLButtonElement | null | undefined) =>
    element &&
    !element.disabled &&
    !element.closest('[inert], [hidden]') &&
    (!modal || modal.contains(element)) &&
    element.getClientRects().length > 0 &&
    getComputedStyle(element).visibility !== 'hidden';
  const siblings = Array.from(
    doc.querySelectorAll<HTMLButtonElement>('[data-workspace-tab-open]'),
  ).filter((button) => button.dataset.paneIndex === String(plan.paneIndex) && usable(button));
  const pane = doc.querySelectorAll('.content-pane')[plan.paneIndex];
  const candidates = [
    siblings[Math.min(plan.position, siblings.length - 1)],
    doc.querySelector<HTMLButtonElement>('.workspace-tab.active [data-workspace-tab-open]'),
    doc.querySelector<HTMLButtonElement>('.workspace-tabs-new'),
    pane?.querySelector<HTMLButtonElement>('.empty-state button'),
    pane?.querySelector<HTMLButtonElement>('.pane-toolbar button:not(:disabled)'),
  ];
  candidates.find(usable)?.focus({ preventScroll: true });
}

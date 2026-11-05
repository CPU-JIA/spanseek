export type ColumnWidths = { sidebar_width: number | null; note_list_width: number | null };
export const SIDEBAR_MIN = 160;
export const SIDEBAR_MAX = 400;
export const LIST_MIN = 200;
export const LIST_MAX = 600;
export const EDITOR_MIN = 320;
export const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), Math.max(min, max));

/** Pane-host padding, pane borders and the gap between two panes. */
export const paneChrome = (sidebarVisible: boolean, panes = 1) =>
  (sidebarVisible ? 8 : 16) + panes * 2 + (panes - 1) * 8;

/** Fit preferred sizes to this window without changing the stored preference. */
export function fitColumns(
  width: number,
  sidebarVisible: boolean,
  listVisible: boolean,
  preferred: ColumnWidths,
) {
  const sidebarAllowed = width >= SIDEBAR_MIN + EDITOR_MIN + paneChrome(true);
  sidebarVisible &&= sidebarAllowed;
  const listAllowed =
    width >=
    (sidebarVisible ? SIDEBAR_MIN : 0) + LIST_MIN + EDITOR_MIN + paneChrome(sidebarVisible) + 1;
  listVisible &&= listAllowed;
  let sidebar = sidebarVisible ? (preferred.sidebar_width ?? (width <= 780 ? 192 : 256)) : 0;
  let list = listVisible
    ? (preferred.note_list_width ??
      (width <= 780 ? 210 : width <= 1000 ? 235 : width <= 1280 ? 255 : 292))
    : 0;
  sidebar = sidebarVisible ? clamp(sidebar, SIDEBAR_MIN, SIDEBAR_MAX) : 0;
  list = listVisible ? clamp(list, LIST_MIN, LIST_MAX) : 0;
  // The sidebar resizer overlaps its neighbour; only the list resizer consumes width.
  const chrome = paneChrome(sidebarVisible) + Number(listVisible);
  const budget = width - EDITOR_MIN - chrome;
  if (sidebar + list > budget && listVisible) list = Math.max(LIST_MIN, budget - sidebar);
  if (sidebar + list > budget && sidebarVisible) sidebar = Math.max(SIDEBAR_MIN, budget - list);
  return {
    sidebar,
    list,
    sidebarAllowed,
    listAllowed,
    sidebarMax: Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, width - list - EDITOR_MIN - chrome)),
    listMax: Math.max(LIST_MIN, Math.min(LIST_MAX, width - sidebar - EDITOR_MIN - chrome)),
  };
}

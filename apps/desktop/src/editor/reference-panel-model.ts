/**
 * Placeholders for the backlink panel.
 *
 * Refreshing the panel empties its rows before the new page arrives, so without
 * placeholders the section collapses to a single line and jumps back when the
 * rows land. Loading *more* rows keeps the rows already on screen, so nothing
 * is replaced there - only a first load draws the skeleton.
 */

export const REFERENCE_SKELETON_ROWS = 3;

export function referenceSkeletonRows(state: { loading: boolean; rows: number }): number {
  return state.loading && state.rows === 0 ? REFERENCE_SKELETON_ROWS : 0;
}

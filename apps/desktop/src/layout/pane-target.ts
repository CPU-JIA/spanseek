/**
 * Resolve the pane a navigation should use.
 *
 * Session state and delayed clicks can briefly disagree after a split pane is
 * closed. Falling back to the active pane (then the first pane) keeps a valid
 * reference/navigation action inside the window instead of reporting that a
 * pane was closed while one is plainly available.
 */
export function resolvePaneIndex(
  paneCount: number,
  requested: number | null | undefined,
  active: number | null | undefined,
): number | null {
  if (!Number.isInteger(paneCount) || paneCount < 1) return null;
  for (const candidate of [requested, active, 0]) {
    if (
      candidate !== null &&
      candidate !== undefined &&
      Number.isInteger(candidate) &&
      candidate >= 0 &&
      candidate < paneCount
    ) {
      return candidate;
    }
  }
  return null;
}

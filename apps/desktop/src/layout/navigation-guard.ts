type NavigationStamp = { workspaceKey: string; sequence: number; epoch: number };
export function navigationIsCurrent(
  expected: NavigationStamp,
  actual: NavigationStamp & {
    sessionWorkspaceKey: string;
    panePresent: boolean;
    transitioning: boolean;
  },
) {
  return (
    !!expected.workspaceKey &&
    actual.workspaceKey === expected.workspaceKey &&
    actual.sessionWorkspaceKey === expected.workspaceKey &&
    actual.sequence === expected.sequence &&
    actual.epoch === expected.epoch &&
    actual.panePresent &&
    !actual.transitioning
  );
}

/** Recheck after every asynchronous save; a later navigation owns the pane. */
export async function navigateAfterBarriers(
  barriers: Array<() => Promise<unknown>>,
  current: () => boolean,
  commit: () => Promise<unknown>,
): Promise<boolean> {
  if (!current()) return false;
  for (const barrier of barriers) {
    await barrier();
    if (!current()) return false;
  }
  await commit();
  return current();
}

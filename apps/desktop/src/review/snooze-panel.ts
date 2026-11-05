/**
 * The 稍后回顾 panel remembers the date *and the entry the date was typed for*.
 * A date can therefore never be submitted for another entry: switching the
 * selection closes the panel instead of carrying the previous item's date over,
 * and reopening the same entry keeps what was already typed.
 */
export type SnoozePanel = { entryId: string; date: string };

/** What the entry currently on screen shows. Every other entry sees a closed panel. */
export type SnoozeView = { open: boolean; date: string };

export function reviewSnoozeView(panel: SnoozePanel | null, entryId: string | null): SnoozeView {
  if (panel === null || entryId === null || panel.entryId !== entryId)
    return { open: false, date: '' };
  return { open: true, date: panel.date };
}

/** The 稍后回顾 button: the same entry closes the panel, any other entry starts empty. */
export function toggleSnoozePanel(panel: SnoozePanel | null, entryId: string): SnoozePanel | null {
  return panel?.entryId === entryId ? null : { entryId, date: '' };
}

/** The date the field reports; it stays bound to the entry it was typed for. */
export function withSnoozeDate(
  panel: SnoozePanel | null,
  entryId: string,
  date: string,
): SnoozePanel {
  return panel?.entryId === entryId ? { ...panel, date } : { entryId, date };
}

const two = (value: number) => String(value).padStart(2, '0');

/**
 * `min` for the datetime-local field, in the local wall clock both the field and
 * `snoozeTimestamp` read. It is the *next* minute on purpose: a time equal to the
 * current minute is not in the future, so `snoozeTimestamp` refuses it and
 * offering it as a valid minimum would be a trap.
 */
export function snoozeMinimum(now: Date): string {
  const next = new Date(Math.floor(now.getTime() / 60000) * 60000 + 60000);
  return [
    next.getFullYear(),
    '-',
    two(next.getMonth() + 1),
    '-',
    two(next.getDate()),
    'T',
    two(next.getHours()),
    ':',
    two(next.getMinutes()),
  ].join('');
}

/** Escape leaves the panel; every other key keeps the field's own behaviour (Enter submits). */
export function snoozeKeyAction(key: string): 'cancel' | null {
  return key === 'Escape' ? 'cancel' : null;
}

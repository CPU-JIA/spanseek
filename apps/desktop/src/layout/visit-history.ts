export type Visit<T> = { id: number; key: string; state: T };

/** Window-local navigation only: it never stores or restores document contents. */
export class VisitHistory<T> {
  private entries: Visit<T>[] = [];
  private index = -1;
  private nextId = 0;

  observe(key: string, state: T, replaceCurrent = false) {
    const current = this.current();
    if (current && (current.key === key || replaceCurrent)) {
      this.entries[this.index] = { ...current, key, state };
      return;
    }
    this.entries = this.entries.slice(0, this.index + 1);
    this.entries.push({ id: ++this.nextId, key, state });
    if (this.entries.length > 100) this.entries.shift();
    this.index = this.entries.length - 1;
  }

  current(): Visit<T> | undefined {
    return this.entries[this.index];
  }

  peek(direction: -1 | 1): Visit<T> | undefined {
    return this.entries[this.index + direction];
  }

  /** Advance only after the caller's save barriers and destination checks succeed. */
  accept(id: number): boolean {
    const index = this.entries.findIndex((entry) => entry.id === id);
    if (index < 0) return false;
    this.index = index;
    return true;
  }
}

type PaneLocation = {
  view: string;
  panes: Array<{ object_id: string; kind: string } | null>;
  activePane: number;
  gallery: boolean;
  inbox: unknown;
  sourceLocations: Record<number, unknown>;
  referenceLocations: Record<number, { target: unknown; occurrenceId?: string }>;
};

/** Resizing the workspace or focusing a pane is not a new content destination. */
export function isPaneLayoutChange(
  previous: PaneLocation | undefined,
  next: PaneLocation,
): boolean {
  if (
    !previous ||
    previous.view !== next.view ||
    (next.view !== 'notes' && next.view !== 'reader') ||
    previous.gallery !== next.gallery ||
    previous.inbox !== next.inbox ||
    (previous.panes.length === next.panes.length && previous.activePane === next.activePane)
  )
    return false;
  const equal = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
  const reference = (location: PaneLocation, index: number) => {
    const value = location.referenceLocations[index];
    return [value?.target, value?.occurrenceId];
  };
  for (let index = 0; index < Math.min(previous.panes.length, next.panes.length); index++) {
    if (
      !equal(previous.panes[index], next.panes[index]) ||
      !equal(previous.sourceLocations[index], next.sourceLocations[index]) ||
      !equal(reference(previous, index), reference(next, index))
    )
      return false;
  }
  // Split duplicates an existing document. Opening a different document (or
  // source version/location) remains a real visit even if a pane was also added.
  return next.panes.slice(previous.panes.length).every((tab, offset) => {
    const index = previous.panes.length + offset;
    return previous.panes.some(
      (old, oldIndex) =>
        equal(old, tab) &&
        (next.sourceLocations[index] === undefined ||
          equal(previous.sourceLocations[oldIndex], next.sourceLocations[index])) &&
        (next.referenceLocations[index] === undefined ||
          equal(reference(previous, oldIndex), reference(next, index))),
    );
  });
}

export function navigationKey(event: {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  isComposing: boolean;
  keyCode?: number;
}): -1 | 1 | null {
  if (
    event.isComposing ||
    event.keyCode === 229 ||
    event.ctrlKey ||
    event.metaKey ||
    event.shiftKey
  )
    return null;
  if (event.key === 'BrowserBack' || (event.altKey && event.key === 'ArrowLeft')) return -1;
  if (event.key === 'BrowserForward' || (event.altKey && event.key === 'ArrowRight')) return 1;
  return null;
}

export function mouseNavigation(button: number): -1 | 1 | null {
  return button === 3 ? -1 : button === 4 ? 1 : null;
}

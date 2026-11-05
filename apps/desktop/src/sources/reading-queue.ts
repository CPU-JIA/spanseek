import type { PdfReadingView, ReadingState, SourceLocator } from '@pkms/contracts';

export type CapturedReadingPosition = {
  locator: SourceLocator;
  view?: PdfReadingView | null;
  locatorReady?: boolean;
};

/** Latest foreground reading position, serialized across panes and close barriers. */
export class ReadingQueue {
  private owner: string | null = null;
  private version: string | null = null;
  private pending: {
    locator: SourceLocator;
    view: PdfReadingView | null;
    locatorReady: boolean;
    sequence: number;
  } | null = null;
  private captures = new Map<string, () => CapturedReadingPosition | null>();
  private sequence = 0;
  private failed = false;
  private flight: Promise<void> | null = null;
  private load: () => Promise<ReadingState>;
  private save: (
    state: ReadingState,
    locator: SourceLocator,
    view: PdfReadingView | null,
    locatorReady: boolean,
  ) => Promise<void>;
  constructor(
    load: () => Promise<ReadingState>,
    save: (
      state: ReadingState,
      locator: SourceLocator,
      view: PdfReadingView | null,
      locatorReady: boolean,
    ) => Promise<void>,
  ) {
    this.load = load;
    this.save = save;
  }
  /** Mounting a background pane must not displace the last interacting pane. */
  register(owner: string, version: string, capture?: () => CapturedReadingPosition | null) {
    if (capture) this.captures.set(owner, capture);
    if (this.owner === null || this.owner === owner) this.activate(owner, version);
  }
  release(owner: string) {
    if (this.owner === owner) {
      this.capture();
      this.owner = null;
      this.version = null;
    }
    this.captures.delete(owner);
    // Pending work belongs to the source and remains available to close barriers.
  }
  activate(owner: string, version: string) {
    this.owner = owner;
    this.version = version;
  }
  record(
    owner: string,
    locator: SourceLocator,
    view: PdfReadingView | null = null,
    locatorReady = true,
  ) {
    if (owner !== this.owner || locator.source_version_id !== this.version) return;
    if (
      JSON.stringify(this.pending?.locator) === JSON.stringify(locator) &&
      JSON.stringify(this.pending?.view) === JSON.stringify(view) &&
      this.pending?.locatorReady === locatorReady
    )
      return;
    this.pending = {
      locator: structuredClone(locator),
      view: structuredClone(view),
      locatorReady,
      sequence: ++this.sequence,
    };
    this.failed = false;
  }
  private capture() {
    if (!this.owner) return;
    const position = this.captures.get(this.owner)?.();
    if (position)
      this.record(
        this.owner,
        position.locator,
        position.view ?? null,
        position.locatorReady ?? true,
      );
  }
  hasPending() {
    return this.pending !== null;
  }
  needsAutoFlush() {
    return this.pending !== null && !this.failed;
  }
  flush = (): Promise<void> => {
    // A normal close can precede the next animation frame or scroll event.
    // Read the active view synchronously before the durable write barrier begins.
    this.capture();
    if (this.flight) return this.flight.then(() => (this.pending ? this.flush() : undefined));
    this.flight = (async () => {
      while (this.pending) {
        const position = this.pending;
        const state = await this.load();
        if (
          state.status === 'unread' ||
          state.source_version_id !== position.locator.source_version_id ||
          JSON.stringify(state.locator) !==
            JSON.stringify(position.locatorReady ? position.locator : null) ||
          JSON.stringify(state.pdf_view ?? null) !== JSON.stringify(position.view)
        )
          await this.save(state, position.locator, position.view, position.locatorReady);
        if (this.pending?.sequence === position.sequence) this.pending = null;
      }
    })()
      .catch((error) => {
        this.failed = true;
        throw error;
      })
      .finally(() => {
        this.flight = null;
      });
    // A new position can arrive after the drain loop exits but before cleanup.
    // Every close barrier must cover that position, including the initiating caller.
    return this.flight.then(() => (this.pending ? this.flush() : undefined));
  };
}

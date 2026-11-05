export const zoomLevels = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3];

export function nextZoom(current: number, direction: -1 | 1): number {
  return direction === 1
    ? (zoomLevels.find((value) => value > current + 0.001) ?? zoomLevels.at(-1)!)
    : (zoomLevels.filter((value) => value < current - 0.001).at(-1) ?? zoomLevels[0]);
}

/** Serialize native zoom writes, coalescing a wheel burst to its most recent target. */
export class WindowZoom {
  private desired = 1;
  private applied: number | null = null;
  private running: Promise<void> | null = null;
  private wheelDelta = 0;
  private write: (scale: number) => Promise<void>;
  constructor(write: (scale: number) => Promise<void>) {
    this.write = write;
  }

  reset(scale = 1) {
    this.wheelDelta = 0;
    return this.set(scale);
  }
  /** The scale this instance is heading for, including a queued write. */
  current() {
    return this.desired;
  }
  step(direction: -1 | 1) {
    return this.set(nextZoom(this.desired, direction));
  }
  wheel(delta: number, mode: number) {
    const pixels = delta * (mode === 1 ? 40 : mode === 2 ? 100 : 1);
    if (!Number.isFinite(pixels) || pixels === 0) return Promise.resolve();
    if (Math.sign(pixels) !== Math.sign(this.wheelDelta)) this.wheelDelta = 0;
    this.wheelDelta += pixels;
    if (Math.abs(this.wheelDelta) < 100) return Promise.resolve();
    const direction = this.wheelDelta < 0 ? 1 : -1;
    this.wheelDelta = 0;
    return this.step(direction);
  }
  private set(scale: number): Promise<void> {
    this.desired = scale;
    if (this.running)
      return this.running.then(() =>
        this.applied === this.desired ? undefined : this.set(this.desired),
      );
    this.running = Promise.resolve()
      .then(async () => {
        while (this.applied !== this.desired) {
          const target = this.desired;
          try {
            await this.write(target);
            this.applied = target;
          } catch (error) {
            this.desired = this.applied ?? 1;
            throw error;
          }
        }
      })
      .finally(() => {
        this.running = null;
      });
    return this.running;
  }
}

/** Query values may return to A after B; their request ownership must not. */
export class SourceListEpoch {
  private epoch = 0;
  begin() {
    return ++this.epoch;
  }
  current() {
    return this.epoch;
  }
  isCurrent(epoch: number) {
    return epoch === this.epoch;
  }
}

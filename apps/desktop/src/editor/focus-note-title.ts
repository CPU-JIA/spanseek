export function focusActiveNoteTitle(options: { select?: boolean } = {}, attempt = 0): void {
  const title = document.querySelector<HTMLInputElement>(
    '.content-pane.active-pane input.note-title',
  );
  if (title) {
    title.focus({ preventScroll: true });
    if (options.select) title.select();
    return;
  }
  if (attempt < 20) {
    window.setTimeout(() => focusActiveNoteTitle(options, attempt + 1), 25);
  }
}

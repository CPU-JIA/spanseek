/**
 * The keyboard shortcuts the kernel lets a user choose.
 *
 * `validate_preferences` in `crates/pkms-app/src/state.rs` accepts exactly this
 * set, so the picker can only offer values the core will store. A shortcut is
 * stored as the modifier-plus-key text below and matched against real key
 * events here, which keeps one source for what the interface shows and what it
 * does.
 */
export const SHORTCUT_OPTIONS = [
  'Ctrl+N',
  'Ctrl+Shift+N',
  'Ctrl+K',
  'Ctrl+P',
  'Ctrl+Shift+K',
  'Alt+N',
  'Alt+K',
] as const;

export type Shortcut = (typeof SHORTCUT_OPTIONS)[number];

export const DEFAULT_NEW_NOTE_SHORTCUT: Shortcut = 'Ctrl+N';
export const DEFAULT_SEARCH_SHORTCUT: Shortcut = 'Ctrl+K';

type Modifiers = { ctrl: boolean; shift: boolean; alt: boolean };

function parse(spec: string): { modifiers: Modifiers; key: string } | null {
  const parts = spec.split('+').map((part) => part.trim());
  const key = parts.pop();
  if (!key) return null;
  const modifiers: Modifiers = { ctrl: false, shift: false, alt: false };
  for (const part of parts) {
    if (part === 'Ctrl' || part === 'Cmd') modifiers.ctrl = true;
    else if (part === 'Shift') modifiers.shift = true;
    else if (part === 'Alt') modifiers.alt = true;
    else return null;
  }
  return { modifiers, key: key.toLowerCase() };
}

/**
 * True when `event` is exactly `spec`. An exact match matters: a stored
 * `Ctrl+N` must not also fire on `Ctrl+Shift+N`, and a shortcut must not steal
 * a key while an input method is composing.
 */
export function matchShortcut(spec: string, event: KeyboardEvent): boolean {
  const parsed = parse(spec);
  if (!parsed || event.isComposing || event.keyCode === 229) return false;
  // Meta counts as Ctrl so a mac keyboard behaves the same as on Windows.
  const ctrl = event.ctrlKey || event.metaKey;
  return (
    ctrl === parsed.modifiers.ctrl &&
    event.shiftKey === parsed.modifiers.shift &&
    event.altKey === parsed.modifiers.alt &&
    event.key.toLowerCase() === parsed.key
  );
}

/** Human-readable parts for a `<kbd>` row: `Ctrl+N` becomes `['Ctrl', 'N']`. */
export function shortcutParts(spec: string): string[] {
  return spec.split('+').map((part) => part.trim());
}

/** A stored value that is not in the current whitelist falls back to the default. */
export function normalizedShortcut(value: string | undefined, fallback: Shortcut): Shortcut {
  return (SHORTCUT_OPTIONS as readonly string[]).includes(value ?? '')
    ? (value as Shortcut)
    : fallback;
}

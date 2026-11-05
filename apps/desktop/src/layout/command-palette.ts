import type { ReactNode } from 'react';

export type CommandPaletteItem = {
  id: string;
  label: string;
  description?: string;
  keywords?: readonly string[];
  icon?: ReactNode;
  shortcut?: string;
  disabled?: boolean;
  run: () => void | Promise<void>;
};

/** An IME owns the key while composition is active, including the closing key. */
export function shouldIgnoreCommandPaletteKey(
  event: { isComposing?: boolean; keyCode?: number },
  composing = false,
): boolean {
  return composing || event.isComposing === true || event.keyCode === 229;
}

/** Keep a visible selection valid when filtering shrinks the list in the same render. */
export function clampCommandPaletteIndex(index: number, length: number): number {
  if (length <= 0) return -1;
  return Math.min(Math.max(index, 0), length - 1);
}

/** Keep filtering deterministic so the keyboard index and visible rows share one source. */
export function filterCommandPalette(
  items: readonly CommandPaletteItem[],
  query: string,
): CommandPaletteItem[] {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return [...items];
  return items.filter((item) =>
    [item.label, item.description, ...(item.keywords ?? [])]
      .filter(Boolean)
      .join('\n')
      .toLocaleLowerCase()
      .includes(normalized),
  );
}

/** Arrow keys wrap at the ends; an empty result has no active row. */
export function moveCommandPaletteIndex(index: number, delta: number, length: number): number {
  if (length <= 0) return -1;
  if (index < 0) return delta > 0 ? 0 : length - 1;
  return (index + delta + length) % length;
}

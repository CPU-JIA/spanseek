/**
 * Dismissal and focus rules of the layout drawer, kept outside the component so
 * they can be exercised without a DOM.
 */

export type DrawerCancelState = {
  /** An IME composition is in flight inside the drawer. */
  composing: boolean;
};

/**
 * A native modal dialog closes itself on Escape after a `cancel` event, so that
 * event — not the keydown handler — is the only place that can veto the close.
 * While an IME is composing, Escape belongs to the candidate window.
 */
export function shouldBlockDrawerCancel({ composing }: DrawerCancelState): boolean {
  return composing;
}

export type DrawerFocusState = {
  /** The element that had focus when the drawer opened; null when it was not an element. */
  previous: { isConnected: boolean } | null;
  /** Focus fell back to the document body after the drawer closed. */
  activeIsBody: boolean;
  /** The closed drawer still holds focus. */
  activeInsideDrawer: boolean;
};

/** Give focus back to the opener only when nothing else claimed it in the meantime. */
export function shouldRestoreDrawerFocus({
  previous,
  activeIsBody,
  activeInsideDrawer,
}: DrawerFocusState): boolean {
  return Boolean(previous?.isConnected) && (activeIsBody || activeInsideDrawer);
}

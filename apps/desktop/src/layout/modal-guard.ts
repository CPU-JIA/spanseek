/**
 * The guards that stop keyboard shortcuts and navigation from reaching the UI
 * behind an open modal, written once so they cannot drift apart.
 *
 * Every modal in the app is a native `<dialog>` opened with `showModal()`, so
 * `dialog[open]` is what matches today. The `[role="dialog"][aria-modal="true"]`
 * half matches nothing yet: it is the WAI-ARIA contract a future non-native modal
 * has to satisfy, and keeping it means such a modal cannot silently bypass the
 * guards. It is deliberately *not* satisfied by adding `aria-modal` to a native
 * dialog, whose modality is already implicit.
 */
const NATIVE_MODAL = 'dialog[open]';
const ARIA_MODAL = '[role="dialog"][aria-modal="true"]';

/** Any modal, drawer included. */
export const modalGuard = `${NATIVE_MODAL}, ${ARIA_MODAL}`;

/**
 * Modals except the layout drawer: the drawer stays a sibling column, so the
 * shortcuts it leaves available (save, settings, ...) must keep working.
 */
export const modalGuardOutsideDrawer = `dialog[open]:not(.layout-drawer), ${ARIA_MODAL}`;

/** Any modal, plus the non-modal popup surfaces that own the keyboard. */
export const shortcutBarrier = `${modalGuard}, [role="menu"], [role="listbox"]`;

/** The same barrier, with the drawer and without owning menus. */
export const shortcutBarrierOutsideDrawer = `${modalGuardOutsideDrawer}, [role="listbox"]`;

import test from 'node:test';
import assert from 'node:assert/strict';
import { shouldBlockDrawerCancel, shouldRestoreDrawerFocus } from '../src/layout/drawer-policy.ts';

/**
 * The drawer is a native `<dialog>` shown with `showModal()`, so Escape closes it
 * through the `cancel` event and a keydown listener cannot veto it. Composition is
 * tracked through `compositionstart`/`compositionend` and read from there; the
 * real WebView behaviour of Escape during composition still needs a machine check.
 */
test('Escape during an IME composition does not dismiss the drawer', () => {
  assert.equal(shouldBlockDrawerCancel({ composing: true }), true);
  assert.equal(shouldBlockDrawerCancel({ composing: false }), false);
});

test('focus returns to the opener only when nothing else claimed it', () => {
  const opener = { isConnected: true };
  assert.equal(
    shouldRestoreDrawerFocus({ previous: opener, activeIsBody: false, activeInsideDrawer: true }),
    true,
  );
  assert.equal(
    shouldRestoreDrawerFocus({ previous: opener, activeIsBody: true, activeInsideDrawer: false }),
    true,
  );
  assert.equal(
    shouldRestoreDrawerFocus({ previous: opener, activeIsBody: true, activeInsideDrawer: true }),
    true,
  );
  assert.equal(
    shouldRestoreDrawerFocus({ previous: opener, activeIsBody: false, activeInsideDrawer: false }),
    false,
  );
  assert.equal(
    shouldRestoreDrawerFocus({
      previous: { isConnected: false },
      activeIsBody: true,
      activeInsideDrawer: false,
    }),
    false,
  );
  assert.equal(
    shouldRestoreDrawerFocus({ previous: null, activeIsBody: true, activeInsideDrawer: true }),
    false,
  );
});

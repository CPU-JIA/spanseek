import test from 'node:test';
import assert from 'node:assert/strict';
import {
  modalGuard,
  modalGuardOutsideDrawer,
  shortcutBarrier,
  shortcutBarrierOutsideDrawer,
} from '../src/layout/modal-guard.ts';

/**
 * These strings are the only thing standing between an open modal and the
 * shortcuts/navigation behind it, so they are locked here: a future editor that
 * "simplifies" them by dropping the ARIA half would silently reopen the hole the
 * project fixed in V18-G09. There is no DOM in the test runner, so the assertions
 * describe the contract rather than match elements.
 */
const ARIA_MODAL = '[role="dialog"][aria-modal="true"]';

test('every guard still matches the native modal and the ARIA modal contract', () => {
  for (const guard of [
    modalGuard,
    modalGuardOutsideDrawer,
    shortcutBarrier,
    shortcutBarrierOutsideDrawer,
  ]) {
    assert.ok(guard.includes('dialog[open]'), `${guard} must match a native open dialog`);
    assert.ok(guard.includes(ARIA_MODAL), `${guard} must keep the future ARIA modal contract`);
  }
});

test('the drawer stays open to shortcuts while real modals block them', () => {
  assert.ok(!modalGuard.includes('.layout-drawer'));
  assert.ok(modalGuardOutsideDrawer.includes(':not(.layout-drawer)'));
  assert.ok(shortcutBarrier.includes('[role="menu"]'));
  assert.ok(!shortcutBarrierOutsideDrawer.includes('[role="menu"]'));
  assert.ok(shortcutBarrierOutsideDrawer.includes('[role="listbox"]'));
});

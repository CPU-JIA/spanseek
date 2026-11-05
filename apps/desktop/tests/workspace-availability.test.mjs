import test from 'node:test';
import assert from 'node:assert/strict';
import {
  workspaceAvailabilityLabel,
  workspaceAvailabilityText,
  workspaceIsAvailable,
} from '../src/welcome/workspace-availability.ts';

// The Chinese sentence is enough to compare reasons: the English one is asserted
// only for being non-empty.
const translate = (zh, en) => {
  assert.ok(en.length > 0, 'every reason carries an English sentence');
  return zh;
};

test('a recent workspace names each reason it cannot be opened', () => {
  assert.equal(workspaceIsAvailable('available'), true);
  assert.equal(workspaceAvailabilityText('available', translate), null);
  assert.equal(workspaceAvailabilityLabel('available', translate), '打开');

  const reasons = ['missing', 'foreign_library', 'unreadable'].map((value) => {
    assert.equal(workspaceIsAvailable(value), false, `${value} is not available`);
    const reason = workspaceAvailabilityText(value, translate);
    assert.ok(typeof reason === 'string' && reason.length > 8, `${value} keeps a sentence`);
    assert.notEqual(workspaceAvailabilityLabel(value, translate), '打开');
    return reason;
  });
  assert.equal(new Set(reasons).size, reasons.length, 'each reason reads differently');

  // The two situations the old boolean collapsed ("位置不可用" for both) must not
  // read the same: a missing folder is not a folder that now holds another library.
  assert.notEqual(
    workspaceAvailabilityText('missing', translate),
    workspaceAvailabilityText('foreign_library', translate),
  );

  // A reason code from a newer kernel is reported as unknown, never as available.
  const unknown = workspaceAvailabilityText('a_reason_from_a_newer_kernel', translate);
  assert.match(unknown, /不认识|更新应用/);
  assert.equal(workspaceIsAvailable('a_reason_from_a_newer_kernel'), false);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { navigateAfterBarriers, navigationIsCurrent } from '../src/layout/navigation-guard.ts';
const expected = { workspaceKey: 'instance-A/workspace', sequence: 7, epoch: 2 };
const initial = () => ({
  ...expected,
  sessionWorkspaceKey: expected.workspaceKey,
  panePresent: true,
  transitioning: false,
});
const deferred = () => {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
};
for (const [name, change] of [
  ['restored workspace instance', { workspaceKey: 'instance-B/workspace' }],
  ['newer pane navigation', { sequence: 8 }],
  ['another application view', { epoch: 3 }],
  ['closed pane', { panePresent: false }],
  ['workspace transition', { transitioning: true }],
  ['replaced layout session', { sessionWorkspaceKey: 'instance-B/workspace' }],
]) {
  test(`tab close does not commit after a delayed save and ${name}`, async () => {
    const state = initial(),
      saving = deferred();
    let readingWrites = 0,
      layoutWrites = 0;
    const operation = navigateAfterBarriers(
      [
        () => saving.promise,
        async () => {
          readingWrites++;
        },
      ],
      () => navigationIsCurrent(expected, state),
      async () => {
        layoutWrites++;
      },
    );
    Object.assign(state, change);
    saving.resolve();
    assert.equal(await operation, false);
    assert.equal(readingWrites, 0);
    assert.equal(layoutWrites, 0);
  });
}
test('a navigation during the reading barrier prevents the tab mutation', async () => {
  const state = initial(),
    reading = deferred();
  let enteredReading;
  const entered = new Promise((done) => {
    enteredReading = done;
  });
  let committed = false;
  const operation = navigateAfterBarriers(
    [
      async () => {},
      () => {
        enteredReading();
        return reading.promise;
      },
    ],
    () => navigationIsCurrent(expected, state),
    async () => {
      committed = true;
    },
  );
  await entered;
  state.sequence++;
  reading.resolve();
  assert.equal(await operation, false);
  assert.equal(committed, false);
});
test('successful barriers commit once, but late completion cannot claim navigation ownership', async () => {
  const state = initial(),
    saved = deferred();
  let committed;
  const entered = new Promise((done) => {
    committed = done;
  });
  const operation = navigateAfterBarriers(
    [async () => {}],
    () => navigationIsCurrent(expected, state),
    () => {
      committed();
      return saved.promise;
    },
  );
  await entered;
  state.epoch++;
  saved.resolve();
  assert.equal(await operation, false);
  assert.equal(
    await navigateAfterBarriers(
      [],
      () => true,
      async () => {},
    ),
    true,
  );
});

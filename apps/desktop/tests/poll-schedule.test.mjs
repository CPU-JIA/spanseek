import test from 'node:test';
import assert from 'node:assert/strict';
import {
  POLL_ACTIVE_MS,
  POLL_HIDDEN_MS,
  POLL_IDLE_MS,
  POLL_MAX_MS,
  SOURCE_POLL_ACTIVE_MS,
  nextPollDelay,
  nextSourcePollDelay,
  pollOnVisibilityChange,
  sourcePollOnVisibilityChange,
} from '../src/portability/poll-schedule.ts';

const idle = { visible: true, pendingOperations: 0, activeJobs: 0, failures: 0 };

test('a hidden window keeps only a slow heartbeat instead of the fast cadence', () => {
  assert.ok(POLL_ACTIVE_MS < POLL_IDLE_MS && POLL_IDLE_MS < POLL_HIDDEN_MS);
  assert.ok(POLL_HIDDEN_MS <= POLL_MAX_MS);
  assert.equal(nextPollDelay({ ...idle, visible: false }), POLL_HIDDEN_MS);
  assert.equal(nextPollDelay({ ...idle, visible: false, activeJobs: 3 }), POLL_HIDDEN_MS);
  assert.equal(nextSourcePollDelay({ visible: false, activeJobs: 3, failures: 0 }), POLL_HIDDEN_MS);
});

test('becoming visible polls at once while hiding arms the heartbeat', () => {
  assert.deepEqual(pollOnVisibilityChange({ ...idle, visible: true }), {
    pollNow: true,
    delay: POLL_IDLE_MS,
  });
  assert.deepEqual(pollOnVisibilityChange({ ...idle, visible: true, activeJobs: 1 }), {
    pollNow: true,
    delay: POLL_ACTIVE_MS,
  });
  assert.deepEqual(pollOnVisibilityChange({ ...idle, visible: false }), {
    pollNow: false,
    delay: POLL_HIDDEN_MS,
  });
  assert.deepEqual(sourcePollOnVisibilityChange({ visible: true, activeJobs: 1, failures: 0 }), {
    pollNow: true,
    delay: SOURCE_POLL_ACTIVE_MS,
  });
  assert.deepEqual(sourcePollOnVisibilityChange({ visible: false, activeJobs: 1, failures: 0 }), {
    pollNow: false,
    delay: POLL_HIDDEN_MS,
  });
});

test('an idle task center backs off to the idle interval while queued work keeps the old cadence', () => {
  assert.equal(nextPollDelay(idle), POLL_IDLE_MS);
  assert.equal(nextPollDelay({ ...idle, pendingOperations: 1 }), POLL_ACTIVE_MS);
  assert.equal(nextPollDelay({ ...idle, activeJobs: 1 }), POLL_ACTIVE_MS);
  assert.equal(nextPollDelay({ ...idle, pendingOperations: 2, activeJobs: 2 }), POLL_ACTIVE_MS);
});

test('no pending operation and no active job is idle, but one queued job is not', () => {
  assert.equal(
    nextPollDelay({ visible: true, pendingOperations: 0, activeJobs: 0, failures: 0 }),
    10_000,
  );
  assert.equal(
    nextPollDelay({ visible: true, pendingOperations: 0, activeJobs: 1, failures: 0 }),
    1_600,
  );
  assert.equal(
    nextPollDelay({ visible: true, pendingOperations: 1, activeJobs: 0, failures: 0 }),
    1_600,
  );
  assert.equal(nextSourcePollDelay({ visible: true, activeJobs: 0, failures: 0 }), 10_000);
  assert.equal(
    nextSourcePollDelay({ visible: true, activeJobs: 1, failures: 0 }),
    SOURCE_POLL_ACTIVE_MS,
  );
});

test('consecutive poll failures back off up to the cap and recover after one clean cycle', () => {
  const seen = [0, 1, 2, 3, 4, 5, 12].map((failures) =>
    nextPollDelay({ ...idle, activeJobs: 1, failures }),
  );
  assert.deepEqual(seen, [1_600, 3_200, 6_400, 12_800, 25_600, 51_200, 60_000]);
  assert.equal(nextPollDelay({ ...idle, activeJobs: 1, failures: 0 }), POLL_ACTIVE_MS);
  assert.equal(nextPollDelay({ ...idle, failures: 0 }), POLL_IDLE_MS);
  assert.equal(nextPollDelay({ ...idle, visible: false, failures: 0 }), POLL_HIDDEN_MS);
  assert.equal(nextPollDelay({ ...idle, visible: false, failures: 1 }), POLL_MAX_MS);
  assert.equal(nextSourcePollDelay({ visible: true, activeJobs: 1, failures: 0 }), 1_800);
  assert.equal(nextSourcePollDelay({ visible: true, activeJobs: 1, failures: 1 }), 3_600);
});

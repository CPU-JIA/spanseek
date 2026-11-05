import test from 'node:test';
import assert from 'node:assert/strict';
import {
  STARTUP_RETRY_BASE_MS,
  STARTUP_RETRY_CEILING_MS,
  STARTUP_RETRY_HIDDEN_MS,
  STARTUP_RETRY_LIMIT,
  startupRetryDelay,
} from '../src/startup-retry.ts';

test('a starting backend is asked less and less often and then reports failure', () => {
  assert.equal(startupRetryDelay(0, false), STARTUP_RETRY_BASE_MS);
  assert.equal(startupRetryDelay(1, false), STARTUP_RETRY_BASE_MS * 2);
  assert.equal(startupRetryDelay(2, false), STARTUP_RETRY_BASE_MS * 4);
  assert.equal(startupRetryDelay(5, false), STARTUP_RETRY_CEILING_MS);
  assert.equal(startupRetryDelay(STARTUP_RETRY_LIMIT - 1, false), STARTUP_RETRY_CEILING_MS);

  // Never four times a second: the wait grows until the ceiling and stays bounded.
  let previous = 0;
  for (let attempt = 0; attempt < STARTUP_RETRY_LIMIT; attempt += 1) {
    const delay = startupRetryDelay(attempt, false);
    assert.ok(delay >= previous, `attempt ${attempt} never gets faster`);
    assert.ok(
      delay >= STARTUP_RETRY_BASE_MS && delay <= STARTUP_RETRY_CEILING_MS,
      `attempt ${attempt} stays inside the bounds`,
    );
    previous = delay;
  }

  // A hidden window has nobody waiting on it, so it waits much longer.
  assert.equal(startupRetryDelay(6, true), STARTUP_RETRY_HIDDEN_MS);
  assert.ok(startupRetryDelay(6, true) > startupRetryDelay(6, false));

  // A spent budget tells the caller to show the failure instead of polling on.
  assert.equal(startupRetryDelay(STARTUP_RETRY_LIMIT, false), null);
  assert.equal(startupRetryDelay(STARTUP_RETRY_LIMIT + 5, true), null);

  // Nonsense input falls back to the base delay instead of a NaN timer.
  assert.equal(startupRetryDelay(Number.NaN, false), STARTUP_RETRY_BASE_MS);
  assert.equal(startupRetryDelay(-3, false), STARTUP_RETRY_BASE_MS);
});

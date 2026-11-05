import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

/**
 * A request the backend refuses for good leaves the queue, and the queue names it
 * in its own dismissable notice. The task centre used to repeat the very same
 * failure as an unattributed banner line, so one event read as two problems.
 */
const source = await readFile(
  new URL('../src/portability/TaskCenter.tsx', import.meta.url),
  'utf8',
);
/** The retry handler only: cancellation errors are not queue refusals. */
const retry = source.slice(
  source.indexOf('async function retry('),
  source.indexOf('async function cancel('),
);

test('retry reports a failure once, through the notice the queue already wrote', () => {
  assert.match(source, /import \{ actionErrorText \} from '\.\/pending-actions';/);
  assert.ok(retry.length > 0, 'the retry handler must exist for this assertion to mean anything');
  assert.match(retry, /requests\.pending\.find\(\(entry\) => entry\.id === id\)/);
  assert.match(retry, /setActionError\(actionErrorText\(error, queued\)\)/);
  assert.doesNotMatch(
    retry,
    /message\(error\)/,
    'the refused text must not be repeated in the banner',
  );
});

test('the named notice stays rendered, attributed and dismissable', () => {
  assert.match(source, /requests\.notice &&/);
  assert.match(source, /className="task-notice"/);
  assert.match(source, /requests\.notice\.displayName/);
  assert.match(source, /portableActions\.dismissNotice\(\)/);
});

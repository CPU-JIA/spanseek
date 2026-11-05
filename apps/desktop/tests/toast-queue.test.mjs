import test from 'node:test';
import assert from 'node:assert/strict';
import {
  TOAST_LIFETIME_MS,
  TOAST_LIMIT,
  ToastQueue,
  parseDuration,
  toastKey,
} from '../src/ui/toast-queue.ts';

/**
 * Deterministic timers: the queue only ever sees `now`/`schedule`/`cancel`, so
 * the test advances a fake clock instead of sleeping for real seconds.
 */
function fakeClock() {
  let current = 0;
  let sequence = 0;
  const timers = new Map();
  return {
    now: () => current,
    schedule(run, delay) {
      sequence += 1;
      timers.set(sequence, { at: current + delay, run });
      return sequence;
    },
    cancel(handle) {
      timers.delete(handle);
    },
    advance(ms) {
      const target = current + ms;
      for (;;) {
        const due = [...timers.entries()]
          .filter(([, timer]) => timer.at <= target)
          .sort((left, right) => left[1].at - right[1].at)[0];
        if (!due) break;
        const [handle, timer] = due;
        timers.delete(handle);
        current = timer.at;
        timer.run();
      }
      current = target;
    },
    pending: () => timers.size,
  };
}

function createQueue(options = {}) {
  const clock = fakeClock();
  return { clock, queue: new ToastQueue({ ...clock, ...options }) };
}

const visible = (queue) => queue.getSnapshot().visible;
const queued = (queue) => queue.getSnapshot().queued;
const texts = (items) => items.map((item) => item.text);

test('stacks at most three toasts and keeps the rest queued in order', () => {
  const { clock, queue } = createQueue();
  const ids = ['一', '二', '三', '四', '五'].map((text) => queue.notify(text));

  assert.equal(visible(queue).length, TOAST_LIMIT);
  assert.deepEqual(texts(visible(queue)), ['一', '二', '三']);
  assert.deepEqual(texts(queued(queue)), ['四', '五']);
  assert.equal(clock.pending(), TOAST_LIMIT);

  queue.dismiss(ids[0]);
  assert.deepEqual(texts(visible(queue)), ['二', '三', '四']);
  assert.deepEqual(texts(queued(queue)), ['五']);
  assert.equal(clock.pending(), TOAST_LIMIT);
});

test('identical copy is one toast that only gets a fresh countdown', () => {
  const { clock, queue } = createQueue();
  const first = queue.notify('已保存');
  clock.advance(3_000);
  const second = queue.notify('  已保存\n');

  assert.equal(second, first);
  assert.equal(visible(queue).length, 1);
  assert.equal(clock.pending(), 1);

  clock.advance(TOAST_LIFETIME_MS - 1);
  assert.equal(visible(queue).length, 1);
  clock.advance(1);
  assert.equal(visible(queue).length, 0);
});

test('a duplicate of a queued toast does not add a second entry', () => {
  const { queue } = createQueue();
  for (const text of ['一', '二', '三', '四']) queue.notify(text);
  queue.notify('四');

  assert.deepEqual(texts(visible(queue)), ['一', '二', '三']);
  assert.deepEqual(texts(queued(queue)), ['四']);
});

test('success and information messages disappear after the default lifetime', () => {
  const { clock, queue } = createQueue();
  queue.notify({ text: '已导出', tone: 'success' });

  clock.advance(TOAST_LIFETIME_MS - 1);
  assert.equal(visible(queue).length, 1);
  clock.advance(1);
  assert.deepEqual(visible(queue), []);
  assert.equal(clock.pending(), 0);
});

test('hover or focus pauses the countdown and resumes with the remaining time', () => {
  const { clock, queue } = createQueue();
  queue.notify('已保存');
  clock.advance(1_000);

  queue.pauseAll();
  clock.advance(60_000);
  assert.equal(visible(queue).length, 1);
  assert.equal(clock.pending(), 0);

  queue.resumeAll();
  clock.advance(TOAST_LIFETIME_MS - 1_000 - 1);
  assert.equal(visible(queue).length, 1);
  clock.advance(1);
  assert.equal(visible(queue).length, 0);
});

test('a message raised while the host is held starts paused', () => {
  const { clock, queue } = createQueue();
  queue.notify('第一条');
  queue.pauseAll();
  queue.notify('第二条');

  clock.advance(60_000);
  assert.deepEqual(texts(visible(queue)), ['第一条', '第二条']);
  assert.equal(clock.pending(), 0);

  queue.resumeAll();
  clock.advance(TOAST_LIFETIME_MS);
  assert.deepEqual(visible(queue), []);
});

test('a queued toast starts its countdown only when it becomes visible', () => {
  const { clock, queue } = createQueue();
  for (const text of ['一', '二', '三', '四']) queue.notify(text);

  clock.advance(TOAST_LIFETIME_MS);
  assert.deepEqual(texts(visible(queue)), ['四']);
  clock.advance(TOAST_LIFETIME_MS - 1);
  assert.equal(visible(queue).length, 1);
  clock.advance(1);
  assert.equal(visible(queue).length, 0);
});

test('tone, optional close button and per-message lifetime are kept', () => {
  const { clock, queue } = createQueue();
  const plain = queue.notify('已保存');
  const pinned = queue.notify({
    text: '已导出',
    tone: 'success',
    dismissible: false,
    lifetime: 500,
  });

  assert.equal(visible(queue).find((item) => item.id === plain).dismissible, true);
  assert.equal(visible(queue).find((item) => item.id === plain).tone, 'info');
  assert.equal(visible(queue).find((item) => item.id === pinned).tone, 'success');
  assert.equal(visible(queue).find((item) => item.id === pinned).dismissible, false);

  clock.advance(500);
  assert.deepEqual(texts(visible(queue)), ['已保存']);
});

test('the host token sets the default lifetime for later messages', () => {
  const { clock, queue } = createQueue();
  queue.setDefaultLifetime(parseDuration('1s'));

  queue.notify('提示');
  clock.advance(999);
  assert.equal(visible(queue).length, 1);
  clock.advance(1);
  assert.equal(visible(queue).length, 0);
});

test('blank copy is ignored instead of leaving an empty toast', () => {
  const { queue } = createQueue();
  assert.equal(queue.notify('   '), null);
  assert.equal(queue.notify(''), null);
  assert.deepEqual(visible(queue), []);
});

test('subscribers hear only real changes', () => {
  const { queue } = createQueue();
  let calls = 0;
  const stop = queue.subscribe(() => {
    calls += 1;
  });

  queue.notify('一');
  queue.notify('一');
  queue.dismiss('missing');
  assert.equal(calls, 1);

  const id = queue.notify('二');
  assert.equal(calls, 2);

  stop();
  queue.dismiss(id);
  assert.equal(calls, 2);
});

test('clearing the queue cancels every pending countdown', () => {
  const { clock, queue } = createQueue();
  queue.notify('一');
  queue.notify('二');
  queue.clear();

  assert.deepEqual(visible(queue), []);
  assert.equal(clock.pending(), 0);
});

test('duration tokens and copy keys are normalised', () => {
  assert.equal(parseDuration('4000ms'), 4_000);
  assert.equal(parseDuration('4s'), 4_000);
  assert.equal(parseDuration(' 0.5s '), 500);
  assert.equal(parseDuration('0ms'), 0);
  assert.equal(parseDuration(''), null);
  assert.equal(parseDuration(null), null);
  assert.equal(parseDuration('auto'), null);
  assert.equal(parseDuration('4000'), null);
  assert.equal(toastKey('  已  保存 \n'), '已 保存');
});

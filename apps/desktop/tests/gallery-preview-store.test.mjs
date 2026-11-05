import test from 'node:test';
import assert from 'node:assert/strict';
import { PreviewStore, previewKey } from '../src/gallery/preview-store.ts';

const identity = { local_instance_id: 'instance-a', workspace_id: 'workspace' };
const request = (noteId = 'note', changes = {}) => ({
  identity,
  noteId,
  revisionId: `${noteId}-r1`,
  schemaVersion: 2,
  ...changes,
});
const note = (input, body = '本地合成预览') => ({
  id: input.noteId,
  revision_id: input.revisionId,
  schema_version: input.schemaVersion,
  document: {
    type: 'doc',
    content: [{ type: 'paragraph', content: [{ type: 'text', text: body }] }],
  },
});
const tick = () => new Promise((resolve) => setImmediate(resolve));
async function until(predicate) {
  for (let i = 0; i < 50; ++i) {
    if (predicate()) return;
    await tick();
  }
  assert.fail('preview work did not settle');
}
function gate() {
  let resolve, reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function load(store, input) {
  return new Promise((resolve) =>
    store.subscribe(input, (value) => {
      if (value.status !== 'loading') resolve(value);
    }),
  );
}

test('only two reads run, identical fixed revisions deduplicate, and leaving cancels queued reads', async () => {
  const calls = [],
    gates = [];
  const store = new PreviewStore((input) => {
    calls.push(input);
    const next = gate();
    gates.push(next);
    return next.promise;
  });
  const a = [],
    duplicate = [];
  store.subscribe(request('a'), (value) => a.push(value));
  store.subscribe(request('a'), (value) => duplicate.push(value));
  store.subscribe(request('b'), () => {});
  const cancel = store.subscribe(request('c'), (value) => assert.equal(value.status, 'loading'));
  // Loading is a state notification; no native call for this queued entry.
  cancel();
  await tick();
  assert.deepEqual(
    calls.map((value) => value.noteId),
    ['a', 'b'],
  );
  assert.equal(store.stats().running, 2);
  assert.equal(store.stats().queued, 0);
  gates[0].resolve(note(request('a')));
  gates[1].resolve(note(request('b')));
  await until(() => store.stats().running === 0);
  assert.equal(a.at(-1).status, 'ready');
  assert.deepEqual(duplicate.at(-1), a.at(-1));
  assert.equal(calls.length, 2);
});

test('A to B to A subscriptions discard obsolete in-flight results and never seed a stale cache', async () => {
  const reads = [],
    gates = [];
  const store = new PreviewStore((input) => {
    reads.push(input);
    const item = gate();
    gates.push(item);
    return item.promise;
  });
  const old = [],
    fresh = [];
  const a = request('a'),
    b = request('a', { identity: { ...identity, local_instance_id: 'instance-b' } });
  const leaveA = store.subscribe(a, (value) => old.push(value));
  await tick();
  leaveA();
  const leaveB = store.subscribe(b, () => {});
  await tick();
  leaveB();
  store.subscribe(a, (value) => fresh.push(value));
  assert.equal(reads.length, 2);
  gates[0].resolve(note(a, 'obsolete A'));
  await until(() => reads.length === 3);
  assert.equal(store.stats().entries, 0);
  assert.equal(old.length, 1);
  assert.equal(fresh.length, 1);
  gates[1].resolve(note(b, 'obsolete B'));
  gates[2].resolve(note(a, 'current A'));
  await until(() => store.stats().running === 0);
  assert.equal(fresh.at(-1).document.nodes[0].content[0].text, 'current A');
  assert.equal(store.stats().entries, 1);
});

test('errors stay cached across viewport reentry until explicit retry and mismatched revisions cannot render', async () => {
  let reads = 0;
  const input = request();
  const store = new PreviewStore(async () => {
    ++reads;
    return { ...note(input), revision_id: reads === 1 ? 'wrong-revision' : input.revisionId };
  });
  assert.equal((await load(store, input)).status, 'error');
  await until(() => store.stats().running === 0);
  assert.equal((await load(store, input)).status, 'error');
  assert.equal(reads, 1);
  store.retry(input);
  assert.equal((await load(store, input)).status, 'ready');
  assert.equal(reads, 2);
});

test('a stalled preview reports a retryable timeout instead of loading forever', async () => {
  const input = request('stalled');
  let reads = 0;
  const store = new PreviewStore(
    async () => {
      ++reads;
      if (reads === 1) return new Promise(() => {});
      return note(input);
    },
    { timeoutMs: 20 },
  );

  const timedOut = await load(store, input);
  assert.equal(timedOut.status, 'error');
  assert.match(timedOut.error, /timed out/i);
  await until(() => store.stats().running === 0);
  assert.equal(store.stats().entries, 1);

  store.retry(input);
  const recovered = await load(store, input);
  assert.equal(recovered.status, 'ready');
  assert.equal(reads, 2);
});

test('LRU retains only projected fragments and enforces entry and UTF-8 byte bounds', async () => {
  const calls = new Map();
  const store = new PreviewStore(
    async (input) => {
      calls.set(input.noteId, (calls.get(input.noteId) ?? 0) + 1);
      return { ...note(input, '😀'.repeat(10000)), privateFullNoteField: 'must not be retained' };
    },
    { entries: 2, bytes: 12000 },
  );
  const first = await load(store, request('a'));
  assert.ok(first.document.nodes[0].content[0].text.length < 10000);
  assert.equal('privateFullNoteField' in first, false);
  await load(store, request('b'));
  await load(store, request('a'));
  await load(store, request('c'));
  assert.equal(store.stats().entries, 2);
  assert.ok(store.stats().bytes <= 12000);
  await load(store, request('b'));
  assert.equal(calls.get('b'), 2);
  const byteBound = new PreviewStore(async (input) => note(input, '😀'.repeat(1200)), {
    entries: 64,
    bytes: 6000,
  });
  await load(byteBound, request('a'));
  await load(byteBound, request('b'));
  assert.equal(byteBound.stats().entries, 1);
  assert.ok(byteBound.stats().bytes <= 6000);
});

test('cache keys separate each instance, workspace, note, revision and document schema', () => {
  const original = request();
  const variants = [
    original,
    request('other-note'),
    request('note', { identity: { ...identity, local_instance_id: 'other-instance' } }),
    request('note', { identity: { ...identity, workspace_id: 'other-workspace' } }),
    request('note', { revisionId: 'new-revision' }),
    request('note', { schemaVersion: 1 }),
  ];
  assert.equal(new Set(variants.map(previewKey)).size, variants.length);
});

test('a future document schema is rejected even when its revision identity matches', async () => {
  const input = request('future-note', { schemaVersion: 4 });
  const store = new PreviewStore(async () => note(input));
  const result = await load(store, input);
  assert.equal(result.status, 'error');
  assert.match(result.error, /document version cannot be previewed/);
  assert.equal('document' in result, false);
});

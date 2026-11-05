import assert from 'node:assert/strict';
import test from 'node:test';
import { Worker } from 'node:worker_threads';
import { createMarkdownQueue } from '../src/inference/markdown-request.ts';

const request = (source) => ({ kind: 'parse', source, allowFrontmatter: true });
const result = (text) => ({
  document: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] },
  preserved: [],
});

test('message formatting bounds concurrency and discards canceled and late results', async () => {
  const workers = [];
  const queue = createMarkdownQueue(() => {
    const worker = {
      onmessage: null,
      onerror: null,
      onmessageerror: null,
      terminated: false,
      postMessage(value) {
        this.request = value;
      },
      terminate() {
        this.terminated = true;
      },
    };
    workers.push(worker);
    return worker;
  });
  const first = new AbortController(),
    queued = new AbortController();
  const a = queue(request('old response'), first.signal);
  const b = queue(request('kept response'));
  const c = queue(request('hidden history'), queued.signal);
  const d = queue(request('new response'));
  const rejectedA = assert.rejects(a, { name: 'AbortError' });
  const rejectedC = assert.rejects(c, { name: 'AbortError' });
  assert.equal(workers.length, 2);
  const staleReply = workers[0].onmessage;
  queued.abort();
  first.abort();
  staleReply({ data: { value: result('should be discarded') } });
  await Promise.all([rejectedA, rejectedC]);
  await Promise.resolve();
  assert.equal(workers.length, 3);
  assert.equal(workers[0].terminated, true);
  assert.equal(workers[2].request.source, 'new response');
  workers[1].onmessage({ data: { value: result('kept') } });
  workers[2].onmessage({ data: { value: result('new') } });
  assert.deepEqual(await b, result('kept'));
  assert.deepEqual(await d, result('new'));
  assert.ok(workers.every((worker) => worker.terminated));
});

const moduleURL = new URL('../src/inference/markdown-worker.ts', import.meta.url).href;
const bootstrap = `
  import { parentPort } from 'node:worker_threads';
  globalThis.self = { postMessage: (value) => parentPort.postMessage(value) };
  const ready = import(${JSON.stringify(moduleURL)});
  parentPort.on('message', async (data) => { await ready; self.onmessage({ data }); });
`;
function actualWorker() {
  const thread = new Worker(new URL('data:text/javascript,' + encodeURIComponent(bootstrap)));
  const worker = {
    onmessage: null,
    onerror: null,
    onmessageerror: null,
    postMessage: (value) => thread.postMessage(value),
    terminate: () => {
      void thread.terminate();
    },
  };
  thread.on('message', (data) => worker.onmessage?.({ data }));
  thread.on('error', (error) => worker.onerror?.({ message: error.message, preventDefault() {} }));
  return worker;
}

test('chat proposal worker preserves existing identities, prevents footnote collisions, and supports source mode', async () => {
  const queue = createMarkdownQueue(actualWorker);
  const base = {
    type: 'doc',
    content: [
      {
        type: 'paragraph',
        attrs: { id: 'old-block', block_id: 'keep' },
        content: [{ type: 'text', text: '旧正文', marks: [{ type: 'underline' }] }],
      },
      {
        type: 'footnoteDefinition',
        attrs: { id: 'old-footnote', label: 'same' },
        content: [
          {
            type: 'paragraph',
            attrs: { id: 'old-footnote-text' },
            content: [{ type: 'text', text: '旧脚注' }],
          },
        ],
      },
    ],
  };
  const original = structuredClone(base);
  const source = '==高亮== and H<sub>2</sub>O[^same]\n\n[^same]: 新脚注';
  const parsed = await queue({
    kind: 'chat-proposal',
    source,
    base,
    format: 'rich',
    allowFrontmatter: false,
  });
  assert.deepEqual(base, original);
  assert.deepEqual(parsed.document.content.slice(0, base.content.length), original.content);
  const definitions = parsed.document.content.filter((node) => node.type === 'footnoteDefinition');
  assert.equal(definitions.length, 2);
  assert.notEqual(definitions[0].attrs.label, definitions[1].attrs.label);
  assert.match(JSON.stringify(parsed.document), /highlight/);
  assert.match(JSON.stringify(parsed.document), /subscript/);
  const raw = await queue({
    kind: 'chat-proposal',
    source,
    base,
    format: 'source',
    allowFrontmatter: false,
  });
  assert.deepEqual(raw.document.content.slice(0, base.content.length), original.content);
  assert.equal(raw.document.content[base.content.length].content[0].text, source.split('\n')[0]);
});

test('answer proposal worker keeps frozen evidence and reports unsupported syntax without dropping it', async () => {
  const queue = createMarkdownQueue(actualWorker);
  const prepared = await queue({
    kind: 'answer-proposal',
    base: { type: 'doc', content: [] },
    answer: {
      text: '## Heading\n\n$E=mc^2$ and [E1]\n\n![remote](https://example.com/photo.png)',
      request: { search: { query: 'Question' } },
      evidence: [
        {
          citation_id: 'E1',
          evidence: {
            title: 'Saved source',
            target: { object_id: 'object', content_revision: 'frozen', block_id: 'block' },
          },
        },
      ],
    },
  });
  assert.ok(prepared.preserved.includes('image'));
  const nodes = prepared.document.content.flatMap((node) => [node, ...(node.content ?? [])]);
  assert.ok(nodes.some((node) => node.type === 'inlineMath'));
  const reference = nodes.find((node) => node.type === 'noteReference');
  assert.equal(reference.attrs.target_revision_id, 'frozen');
  assert.equal(reference.attrs.target_block_id, 'block');
  assert.match(JSON.stringify(prepared.document), /https:\/\/example.com\/photo.png/);
});

test('worker conversion errors leave the original request available for correction', async () => {
  const queue = createMarkdownQueue(actualWorker);
  const input = request('x'.repeat(4 * 1024 * 1024 + 1));
  await assert.rejects(queue(input), /size limit/);
  assert.equal(input.source.length, 4 * 1024 * 1024 + 1);
  const after = await queue(request('**recovered**'));
  assert.equal(after.document.content[0].content[0].marks[0].type, 'bold');
});

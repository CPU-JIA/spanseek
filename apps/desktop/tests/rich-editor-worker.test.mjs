import assert from 'node:assert/strict';
import test from 'node:test';
import { Worker } from 'node:worker_threads';

const moduleURL = new URL('../src/editor/rich-markdown-worker.ts', import.meta.url).href;
// Run the actual worker module in an isolated JS worker thread. The bridge only
// adapts the browser worker's messaging globals; it does not replace the parser.
const bootstrap = `
  import { parentPort } from 'node:worker_threads';
  globalThis.self = { postMessage: (value) => parentPort.postMessage(value) };
  const ready = import(${JSON.stringify(moduleURL)});
  parentPort.on('message', async (data) => { await ready; self.onmessage({ data }); });
`;
function prepare(source, options = {}) {
  const worker = new Worker(new URL('data:text/javascript,' + encodeURIComponent(bootstrap)));
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      void worker.terminate();
      reject(new Error('Worker timed out'));
    }, 10_000);
    worker.once('message', (value) => {
      clearTimeout(timeout);
      void worker.terminate();
      resolve(value);
    });
    worker.once('error', (error) => {
      clearTimeout(timeout);
      void worker.terminate();
      reject(error);
    });
    worker.postMessage({ source, options });
  });
}

test('Markdown worker converts a large body and honors the existing document context', async () => {
  const source =
    Array.from({ length: 1500 }, (_, index) => `段落 ${index} **粗体**。`).join('\n\n') +
    '\n\n重复块 ^occupied\n\n[^ref]: 脚注\n\n引用[^ref]';
  const result = await prepare(source, {
    allowFrontmatter: false,
    contextDocument: {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          attrs: { id: 'base', block_id: 'occupied' },
          content: [{ type: 'text', text: '原文' }],
        },
      ],
    },
  });
  assert.equal(result.error, undefined);
  assert.ok(result.value.document.content.length >= 1500);
  assert.ok(result.value.preserved.includes('anchor'));
  assert.match(JSON.stringify(result.value.document), /段落 1499/);
  assert.ok(result.value.document.content.some((node) => node.type === 'footnoteDefinition'));
});

test('Markdown worker reports the parser size failure without claiming a converted document', async () => {
  const result = await prepare('x'.repeat(4 * 1024 * 1024 + 1));
  assert.match(result.error, /size limit/);
  assert.equal(result.value, undefined);
});

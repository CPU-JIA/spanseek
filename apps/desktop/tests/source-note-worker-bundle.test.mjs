import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { webcrypto } from 'node:crypto';
import vm from 'node:vm';
import { build } from 'vite';

test('production source-note worker boots without document and decodes Markdown entities', async () => {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const bundle = await build({
    root,
    configFile: fileURLToPath(new URL('../vite.config.ts', import.meta.url)),
    logLevel: 'silent',
    build: {
      write: false,
      rollupOptions: {
        input: fileURLToPath(new URL('./fixtures/source-note-worker-entry.ts', import.meta.url)),
      },
    },
  });
  const outputs = (Array.isArray(bundle) ? bundle : [bundle]).flatMap((entry) => entry.output);
  const worker = outputs.find(
    (entry) => entry.type === 'asset' && /^assets\/source-note-worker-.*\.js$/.test(entry.fileName),
  );
  assert.ok(worker, 'Vite must emit the actual worker artifact');
  const code = worker.type === 'asset' ? String(worker.source) : worker.code;
  const replies = [];
  const self = { postMessage: (message) => replies.push(structuredClone(message)) };
  // Deliberately provide worker globals only. Browser-export module initialization
  // previously failed here at document.createElement before registering onmessage.
  vm.runInNewContext(code, { self, crypto: webcrypto, TextEncoder, TextDecoder, URL });
  assert.equal(typeof self.onmessage, 'function');
  self.onmessage({
    data: {
      format: 'markdown',
      text: '# 中文 &amp; &#x1f600;\n\n**粗体** &copy; &notin; &bogus;\n\n![保留图片](missing.png)',
    },
  });
  assert.equal(replies.length, 1);
  assert.equal(replies[0].error, undefined);
  const rich = replies[0].value;
  assert.equal(rich.document.content[0].type, 'heading');
  assert.equal(rich.document.content[0].content[0].text, '中文 & 😀');
  assert.equal(rich.document.content[1].content[0].marks[0].type, 'bold');
  assert.match(rich.document.content[1].content.map((node) => node.text).join(''), /© ∉ &bogus;/);
  assert.ok(rich.preserved.includes('image'));
  self.onmessage({ data: { format: 'text', text: '# 原样 &amp;\r\n中文😀' } });
  assert.equal(replies[1].error, undefined);
  assert.equal(replies[1].value.document.content[0].content[0].text, '# 原样 &amp;');
  assert.equal(replies[1].value.document.content[1].content[0].text, '中文😀');
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { getSchema } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import { defaultTreeAdapter, parseFragment, serializeOuter } from 'parse5';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

test('copied reference HTML preserves literal labels and pinned targets while regenerating occurrences', async (context) => {
  const server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('..', import.meta.url)),
    cacheDir: await isolatedSsrCache(context),
    logLevel: 'error',
    esbuild: { jsx: 'automatic' },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    appType: 'custom',
  });
  try {
    const { noteReferenceExtension } = await server.ssrLoadModule(
      '/src/editor/reference-extension.tsx',
    );
    const schema = getSchema([
      StarterKit.configure({ trailingNode: false }),
      noteReferenceExtension({ local_instance_id: 'instance', workspace_id: 'workspace' }),
    ]);
    const reference = schema.nodes.noteReference;
    const rule = reference.spec.parseDOM.find((entry) => entry.tag === 'span[data-note-reference]');
    assert.equal(typeof rule?.getAttrs, 'function');
    const target = {
      target_id: '7eb15314-7ea5-4876-ab30-6f4018958928',
      target_block_id: 'c1f5e6f5-fae9-4726-a7e2-a6a86db10440',
      target_revision_id: 'f1e3dcb3-d85a-4d3c-a9f2-9533e1a99506',
    };
    const originalOccurrence = '2b865340-f219-4305-b471-c4e12898cbbe';
    for (const label of ['123', '001', 'true', 'false']) {
      const original = reference.create({ ...target, occurrence_id: originalOccurrence, label });
      const [tag, attributes, text] = reference.spec.toDOM(original);
      assert.equal(tag, 'span');
      assert.equal(text, label);
      const element = defaultTreeAdapter.createElement(
        tag,
        'http://www.w3.org/1999/xhtml',
        Object.entries(attributes)
          .filter(([, value]) => value != null)
          .map(([name, value]) => ({ name, value: String(value) })),
      );
      defaultTreeAdapter.appendChild(element, defaultTreeAdapter.createTextNode(text));
      const html = serializeOuter(element);
      const [span] = parseFragment(html).childNodes;
      assert.equal(span.tagName, 'span');
      assert.equal(
        span.attrs.find((entry) => entry.name === 'occurrence_id').value,
        originalOccurrence,
      );

      // Exercise the real generated ProseMirror rule, including Tiptap's
      // attribute injection. This DOM attribute boundary is not a native
      // clipboard, browser selection, or input-method test.
      const boundary = {
        textContent: span.childNodes.map((child) => child.value).join(''),
        getAttribute: (name) => span.attrs.find((entry) => entry.name === name)?.value ?? null,
      };
      const copies = [rule.getAttrs(boundary), rule.getAttrs(boundary)];
      for (const attributes of copies) {
        assert.equal(attributes.label, label);
        assert.equal(typeof attributes.label, 'string');
        for (const [name, value] of Object.entries(target)) assert.equal(attributes[name], value);
        assert.match(
          attributes.occurrence_id,
          /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
        );
        assert.doesNotThrow(() => reference.createChecked(attributes));
      }
      assert.equal(
        new Set([originalOccurrence, ...copies.map((copy) => copy.occurrence_id)]).size,
        3,
      );
    }
  } finally {
    await server.close();
  }
});

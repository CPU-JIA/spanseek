import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { getSchema } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';
import { PASTE_MARKERS } from '../src/rich/paste-markers.ts';

test('the reading view and the editor share one clipboard contract', async (context) => {
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
    const { WikiLink, WikiEmbed } = await server.ssrLoadModule('/src/editor/rich-schema.ts');
    const { noteReferenceExtension } = await server.ssrLoadModule(
      '/src/editor/reference-extension.tsx',
    );
    const { imageExtension } = await server.ssrLoadModule('/src/editor/NoteEditor.tsx');
    const identity = { local_instance_id: 'instance', workspace_id: 'workspace' };
    const schema = getSchema([
      StarterKit.configure({ trailingNode: false }),
      WikiLink,
      WikiEmbed,
      noteReferenceExtension(identity),
      imageExtension({}, identity),
    ]);
    const nodes = {
      wikiLink: schema.nodes.wikiLink,
      wikiEmbed: schema.nodes.wikiEmbed,
      noteReference: schema.nodes.noteReference,
      image: schema.nodes.image,
    };
    // The exact values the reading view writes for each marker, keyed by the
    // attribute name it emits, plus the node attribute the editor must read back.
    const emission = {
      wikiLink: {
        emitted: { 'data-wiki-target': '笔记 A', label: '显示名' },
        expected: { wiki_target: '笔记 A', label: '显示名' },
      },
      wikiEmbed: {
        emitted: { 'data-wiki-target': '笔记 B', label: '嵌入名' },
        expected: { wiki_target: '笔记 B', label: '嵌入名' },
      },
      noteReference: {
        emitted: {
          target_id: '7eb15314-7ea5-4876-ab30-6f4018958928',
          target_block_id: 'c1f5e6f5-fae9-4726-a7e2-a6a86db10440',
          target_revision_id: 'f1e3dcb3-d85a-4d3c-a9f2-9533e1a99506',
        },
        expected: {
          target_id: '7eb15314-7ea5-4876-ab30-6f4018958928',
          target_block_id: 'c1f5e6f5-fae9-4726-a7e2-a6a86db10440',
          target_revision_id: 'f1e3dcb3-d85a-4d3c-a9f2-9533e1a99506',
        },
        text: '引用标签',
        label: '引用标签',
      },
      image: {
        emitted: {
          blob_hash: 'a'.repeat(64),
          original_name: 'photo.png',
          alt: '说明',
          title: '图片说明',
        },
        expected: {
          blob_hash: 'a'.repeat(64),
          original_name: 'photo.png',
          alt: '说明',
          title: '图片说明',
        },
      },
    };
    for (const [name, marker] of Object.entries(PASTE_MARKERS)) {
      const node = nodes[name];
      assert.ok(node, `${name} must exist in the editor schema`);
      const rules = node.spec.parseDOM ?? [];
      const tags = rules.map((rule) => rule.tag);
      assert.ok(
        tags.includes(marker.tag),
        `${name}: the editor must parse ${marker.tag}, saw ${tags.join(', ')}`,
      );
      const rule = rules.find((entry) => entry.tag === marker.tag);
      assert.equal(
        typeof rule.getAttrs,
        'function',
        `${name}: the marker attributes have to be read back, not defaulted`,
      );
      const values = emission[name].emitted;
      const boundary = {
        textContent: emission[name].text ?? '',
        getAttribute: (attribute) => values[attribute] ?? null,
      };
      const parsed = rule.getAttrs(boundary);
      for (const [attribute, expected] of Object.entries(emission[name].expected)) {
        assert.equal(parsed[attribute], expected, `${name}.${attribute} must survive the paste`);
      }
      assert.doesNotThrow(() => node.createChecked(parsed), `${name} must accept the pasted attrs`);
      for (const attribute of marker.attributes) {
        assert.ok(
          Object.hasOwn(values, attribute),
          `${name}: the test has to cover the emitted "${attribute}" marker`,
        );
      }
    }
  } finally {
    await server.close();
  }
});

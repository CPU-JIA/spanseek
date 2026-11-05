import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';
import {
  assertMermaidSource,
  assertMermaidSvg,
  MERMAID_LIMITS,
} from '../src/rich/render-policy.ts';
import { renderMath } from '../src/rich/math-render.ts';
import { yamlFields, replaceYamlScalar } from '../src/rich/yaml-properties.ts';
import { CALLOUT_ALIASES, CALLOUT_TYPES, getCallout } from '../src/rich/callouts.ts';
import { safeColor, highlightInk } from '../src/rich/colors.ts';
import { footnoteKey } from '../src/rich/footnotes.ts';
import {
  boundedEmbed,
  noteAnchor,
  resolveWiki,
  wikiParts,
  MAX_WIKI_PAGES,
} from '../src/rich/wiki-model.ts';
import { projectPreview, PREVIEW_LIMITS } from '../src/gallery/preview-document.ts';
import { PreviewStore } from '../src/gallery/preview-store.ts';

const text = (value, marks = []) => ({ type: 'text', text: value, marks });
const p = (value, id = 'paragraph') => ({
  type: 'paragraph',
  attrs: { id },
  content: [text(value)],
});
const doc = (...content) => ({ type: 'doc', content });
const note = (id, title, document) => ({
  id,
  title,
  revision: 1,
  revision_id: 'rev',
  parent_revision_id: null,
  content_hash: '',
  author_kind: 'user',
  schema_version: 3,
  document,
  trashed: false,
  updated_at: '',
});
const targetNote = note(
  '11111111-1111-4111-8111-111111111111',
  'Folder/Target',
  doc(
    { type: 'heading', attrs: { id: 'heading', level: 1 }, content: [text('Overview')] },
    { ...p('anchored text', 'block'), attrs: { id: 'block', block_id: 'detail-1' } },
    { type: 'heading', attrs: { id: 'next-heading', level: 1 }, content: [text('Next')] },
    p('outside'),
  ),
);
function port(overrides = {}) {
  return {
    notes: async () => ({
      items: [{ id: targetNote.id, title: targetNote.title }],
      next_cursor: null,
    }),
    sources: async () => ({ items: [], next_cursor: null }),
    note: async (id) => {
      assert.equal(id, targetNote.id);
      return targetNote;
    },
    source: async () => {
      throw Error('unexpected source read');
    },
    version: async () => {
      throw Error('unexpected source version read');
    },
    resolve: async (target) => ({
      target,
      state: 'available',
      target_kind: 'note',
      source_target: null,
    }),
    ...overrides,
  };
}

test('Mermaid rejects configuration and resource requests before running a renderer', () => {
  assert.doesNotThrow(() =>
    assertMermaidSource('flowchart LR\nA[Start] --> B{Choice}\nB --> C[Done]'),
  );
  for (const source of [
    '%%{init: {"securityLevel":"loose"}}%%\ngraph TD; A-->B',
    '---\nconfig:\n  securityLevel: loose\n---\ngraph TD; A-->B',
    'graph TD; click A callback',
    'graph TD\nA@{ img: "local-image.png" }',
    'graph TD\nA[<img src=x onerror=alert(1)>]',
    'graph TD\nA@{ img: "&#x68;ttps://example.test/image.png" }',
    'graph TD\nclassDef remote fill:url(https://example.test/style)',
    'x'.repeat(MERMAID_LIMITS.characters + 1),
  ])
    assert.throws(() => assertMermaidSource(source));
});

test('Mermaid output accepts only local SVG resources and never executable/link elements', () => {
  assert.doesNotThrow(() =>
    assertMermaidSvg(
      '<svg xmlns="http://www.w3.org/2000/svg"><defs><marker id="arrow"/></defs><path style="marker-end:url(#arrow)"/><use href="#arrow"/></svg>',
    ),
  );
  for (const svg of [
    '<svg><script>alert(1)</script></svg>',
    '<svg><foreignObject><div>HTML</div></foreignObject></svg>',
    '<svg><image href="https://example.test/a.png"/></svg>',
    '<svg><a href="#a">x</a></svg>',
    '<svg><use href="&#x68;ttps://example.test/a.svg"/></svg>',
    '<svg onload="alert(1)"></svg>',
    '<svg><style>@import "https://example.test/a";</style></svg>',
  ])
    assert.throws(() => assertMermaidSvg(svg));
});

test('KaTeX produces local HTML/MathML, denies trusted commands and bounds expansion', () => {
  const html = renderMath('x^2 + \\frac{1}{2}', false);
  assert.match(html, /katex/);
  assert.match(html, /<math/);
  assert.throws(() => renderMath('\\href{https://example.test}{click}', false), /源码仍保留/);
  assert.throws(
    () => renderMath('\\includegraphics{https://example.test/a.png}', false),
    /源码仍保留/,
  );
  assert.throws(() => renderMath('\\def\\loop{\\loop}\\loop', false));
  assert.throws(() => renderMath('x'.repeat(12001), false));
});

test('all official callout aliases map without losing unknown type fallback and colors reject CSS', () => {
  assert.equal(CALLOUT_TYPES.length, 13);
  assert.equal(Object.keys(CALLOUT_ALIASES).length, 14);
  assert.equal(getCallout('IMPORTANT').type, 'tip');
  assert.equal(getCallout('error').type, 'danger');
  assert.equal(getCallout('constructor').type, 'note');
  assert.equal(safeColor('#aBcDeF'), '#ABCDEF');
  assert.equal(safeColor('red; background:url(x)'), undefined);
  assert.equal(highlightInk('#000000'), '#FFFFFF');
  assert.equal(highlightInk('#FFFFFF'), '#172033');
});

test('YAML scalar edits retain unrelated nested properties, comments and scalar types byte-for-byte', () => {
  const source =
    '# document comment\ntitle: "Before" # inline\ncount: 3\ndone: false\ndate: 2026-10-09\nnullish: null\ntags:\n  - first # tag comment\n  - second\nnested:\n  keep: [a, {b: c}] # nested comment\n';
  const fields = yamlFields(source);
  assert.equal(fields.error, null);
  assert.deepEqual(
    fields.fields.map((field) => field.kind),
    ['text', 'number', 'boolean', 'date', 'null', 'list', 'nested'],
  );
  const changed = replaceYamlScalar(source, ['title'], 'true');
  assert.equal(changed, source.replace('"Before"', '"true"'));
  assert.equal(yamlFields(changed).fields[0].value, 'true');
  assert.equal(replaceYamlScalar(source, ['tags', 0], 'new'), source.replace('first #', '"new" #'));
  assert.equal(
    replaceYamlScalar(source, ['done'], true),
    source.replace('done: false', 'done: true'),
  );
  const multiline = 'description: | # retained\n  first\n  second\nnext: 1\n';
  assert.equal(
    replaceYamlScalar(multiline, ['description'], 'updated'),
    'description: "updated" # retained\nnext: 1\n',
  );
});

test('YAML never expands aliases, silently fixes duplicate keys or overwrites anchored scalars', () => {
  assert.ok(yamlFields('a: 1\na: 2\n').error);
  const source = 'base: &base value\nother: *base\n';
  assert.deepEqual(
    yamlFields(source).fields.map((field) => field.kind),
    ['nested', 'nested'],
  );
  assert.throws(() => replaceYamlScalar(source, ['base'], 'changed'));
  assert.throws(() => replaceYamlScalar('value: 1\n', ['value'], Infinity));
});

test('wiki paths are confined to logical knowledge names and reject absolute/encoded URLs', () => {
  assert.deepEqual(wikiParts('../Target.md#^a-1', 'Folder/Child/Current'), {
    path: 'Folder/Target.md',
    fragment: '^a-1',
  });
  assert.deepEqual(wikiParts('#Overview'), { path: '', fragment: 'Overview' });
  for (const target of [
    '../../secret',
    'C:\\secret.md',
    '/etc/passwd',
    'https://example.test',
    '%2f%2fexample.test/file',
    'javascript%3Aalert(1)',
    '%',
  ])
    assert.throws(() => wikiParts(target));
});

test('wiki heading and block anchors use actual node IDs and do not pick ambiguous headings', () => {
  const heading = noteAnchor(targetNote.document, 'Overview');
  assert.equal(heading.block, 'heading');
  assert.deepEqual(
    heading.blocks.map((node) => node.attrs.id),
    ['heading', 'block'],
  );
  assert.equal(noteAnchor(targetNote.document, '^detail-1').block, 'block');
  assert.equal(noteAnchor(targetNote.document, '^absent'), null);
  const duplicate = doc(...targetNote.document.content, {
    type: 'heading',
    attrs: { id: 'duplicate', level: 1 },
    content: [text('Overview')],
  });
  assert.equal(noteAnchor(duplicate, 'Overview'), null);
});

test('wiki title lookup checks subsequent pages for ambiguity and respects bound target loss', async () => {
  const available = await resolveWiki({ wiki_target: 'Folder/Target.md#^detail-1' }, null, port());
  assert.equal(available.state, 'available');
  assert.equal(available.target.target_block_id, 'block');
  const ambiguous = await resolveWiki(
    { wiki_target: 'Folder/Target' },
    null,
    port({
      notes: async (_, cursor) =>
        cursor
          ? { items: [{ id: 'second', title: targetNote.title }], next_cursor: null }
          : { items: [{ id: targetNote.id, title: targetNote.title }], next_cursor: 'page-2' },
    }),
  );
  assert.equal(ambiguous.state, 'ambiguous');
  const bound = await resolveWiki(
    { wiki_target: 'Folder/Target', target_id: targetNote.id },
    null,
    port({
      notes: async () => {
        throw Error('must not rebind a lost UUID by title');
      },
      resolve: async () => ({ state: 'target_missing', source_target: null }),
    }),
  );
  assert.equal(bound.state, 'missing');
});

test('bounded wiki lookup does not assert uniqueness before all relevant pages were checked', async () => {
  let pages = 0;
  const result = await resolveWiki(
    { wiki_target: 'Folder/Target' },
    null,
    port({
      notes: async () => ({
        items: [{ id: targetNote.id, title: targetNote.title }],
        next_cursor: String(++pages),
      }),
    }),
  );
  assert.equal(result.state, 'limit');
  assert.equal(pages, MAX_WIKI_PAGES);
});

test('source wiki resolution uses existing source target and preserves unavailable fragment state', async () => {
  const source = { id: 'source', display_title: 'manual.pdf' },
    version = { id: 'version', source_id: 'source', format: 'pdf' };
  const sourceTarget = {
    source_id: 'source',
    source_version_id: 'version',
    parse_id: 'frozen-parse',
    block_offset: 51,
    locator_status: 'exact',
    locator: { kind: 'pdf', source_version_id: 'version', page_index: 3, rects: [[1, 2, 3, 4]] },
    annotation_id: null,
  };
  const io = port({
    sources: async () => ({ items: [source], next_cursor: null }),
    resolve: async () => ({ state: 'available', source_target: sourceTarget }),
    source: async () => source,
    version: async () => version,
  });
  const result = await resolveWiki({ wiki_target: 'manual.pdf' }, null, io);
  assert.equal(result.kind, 'source');
  assert.equal(result.target.target_id, 'source');
  assert.deepEqual(result.sourceTarget, sourceTarget);
  assert.equal(
    (await resolveWiki({ wiki_target: 'manual.pdf#missing' }, null, io)).state,
    'anchor_missing',
  );
  assert.equal(
    (
      await resolveWiki({ wiki_target: 'manual.pdf', target_block_id: 'block' }, null, {
        ...io,
        resolve: async () => ({
          state: 'available',
          source_target: { ...sourceTarget, locator_status: 'unavailable' },
        }),
      })
    ).state,
    'anchor_missing',
  );
});

test('footnote display identity agrees with the core whitespace and case rule', () => {
  assert.equal(footnoteKey('  A\u0085Mixed\tLabel  '), 'a mixed label');
  const preview = projectPreview(
    doc({
      type: 'paragraph',
      content: [
        { type: 'footnoteReference', attrs: { label: 'FIRST' }, marks: [{ type: 'underline' }] },
        {
          type: 'footnoteReference',
          attrs: { label: ' first ' },
          marks: [{ type: 'textStyle', attrs: { color: '#123456' } }],
        },
      ],
    }),
  );
  assert.deepEqual(
    preview.nodes[0].content.map((node) => node.attrs.footnote_number),
    [1, 1],
  );
  assert.deepEqual(preview.nodes[0].content[0].marks, ['underline']);
  assert.equal(preview.nodes[0].content[1].attrs.text_color, '#123456');
});

test('embed budgets stop before unseen branches, preserve Unicode and count source payloads', () => {
  const many = Array.from({ length: 600 }, (_, index) => ({
    type: 'horizontalRule',
    attrs: { id: String(index) },
  }));
  Object.defineProperty(many, 600, {
    get() {
      throw Error('must stop at node budget');
    },
  });
  assert.equal(boundedEmbed(many).content.length, 600);
  assert.equal(boundedEmbed(many).truncated, true);
  const unicode = boundedEmbed([text('😀'.repeat(20000))]);
  assert.equal(unicode.content[0].text.isWellFormed(), true);
  assert.ok(unicode.content[0].text.length <= 24000);
  assert.equal(
    boundedEmbed([{ type: 'blockMath', attrs: { source: 'x'.repeat(24001) } }]).content.length,
    0,
  );
});

test('gallery D3 projection preserves bounded formats without comments, target IDs or live diagrams', () => {
  const source = doc(
    { type: 'heading', attrs: { level: 6 }, content: [text('Six')] },
    {
      type: 'paragraph',
      content: [
        text('Styled', [
          { type: 'underline' },
          { type: 'superscript' },
          { type: 'highlight', attrs: { color: '#FFF2A8' } },
          { type: 'textStyle', attrs: { color: '#123456' } },
        ]),
        { type: 'comment', attrs: { source: 'PRIVATE_COMMENT' } },
      ],
    },
    { type: 'wikiEmbed', attrs: { wiki_target: 'Target', target_id: 'PRIVATE_ID' } },
    {
      type: 'callout',
      attrs: { callout_type: 'tip', title: 'Tip', collapsed: true },
      content: [p('hidden callout')],
    },
    { type: 'codeBlock', attrs: { language: 'mermaid' }, content: [text('graph TD; A-->B')] },
    {
      type: 'table',
      content: [
        {
          type: 'tableRow',
          content: [{ type: 'tableCell', attrs: { align: 'right' }, content: [p('aligned')] }],
        },
      ],
    },
  );
  const preview = projectPreview(source);
  assert.equal(preview.nodes[0].attrs.level, 6);
  assert.equal(preview.nodes[1].content[0].attrs.text_color, '#123456');
  assert.deepEqual(preview.nodes[3].content, []);
  assert.equal(preview.nodes[4].attrs.language, 'mermaid');
  assert.equal(preview.nodes[5].content[0].content[0].attrs.align, 'right');
  assert.doesNotMatch(JSON.stringify(preview), /PRIVATE|hidden callout/);
  assert.ok(
    JSON.stringify(
      projectPreview(doc({ type: 'frontmatter', attrs: { source: 'a'.repeat(100000) } })),
    ).length < PREVIEW_LIMITS.characters,
  );
});

test('gallery store accepts schema 3 and preserves its existing future-version rejection', async () => {
  const input = {
    identity: { local_instance_id: 'local', workspace_id: 'work' },
    noteId: targetNote.id,
    revisionId: targetNote.revision_id,
    schemaVersion: 3,
  };
  const store = new PreviewStore(async () => targetNote);
  const result = await new Promise((resolve) => {
    const release = store.subscribe(input, (state) => {
      if (state.status !== 'loading') {
        release();
        resolve(state);
      }
    });
  });
  assert.equal(result.status, 'ready');
});

test('read renders comments initially concealed while gallery still excludes them, alongside marks, math and footnotes', async (context) => {
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
    const { ReadDocument } = await server.ssrLoadModule('/src/editor/ReadDocument.tsx');
    const { StandaloneDocumentContent } = await server.ssrLoadModule(
      '/src/rich/DocumentContent.tsx',
    );
    const { PreviewDocumentView } = await server.ssrLoadModule(
      '/src/gallery/PreviewDocumentView.tsx',
    );
    const document = doc(
      { type: 'heading', attrs: { id: 'six', level: 6 }, content: [text('Six')] },
      {
        type: 'paragraph',
        content: [
          text('Combo', [
            { type: 'underline' },
            { type: 'subscript' },
            { type: 'highlight', attrs: { color: '#FFF2A8' } },
          ]),
          {
            type: 'inlineMath',
            attrs: { source: 'x^2' },
            marks: [{ type: 'underline' }, { type: 'textStyle', attrs: { color: '#123456' } }],
          },
          { type: 'comment', attrs: { source: 'PRIVATE_COMMENT' } },
          { type: 'footnoteReference', attrs: { label: ' ReF ' }, marks: [{ type: 'bold' }] },
        ],
      },
      {
        type: 'footnoteDefinition',
        attrs: { id: 'fn', label: 'ref' },
        content: [p('Footnote body')],
      },
      {
        type: 'callout',
        attrs: { id: 'ca', callout_type: 'faq', title: 'Question' },
        content: [p('Callout body')],
      },
    );
    const identity = { local_instance_id: 'local', workspace_id: 'work' };
    const html = renderToStaticMarkup(
      createElement(ReadDocument, { note: note('test', 'Title', document), identity }),
    );
    for (const pattern of [
      /<h6/,
      /<u>Combo<\/u>/,
      /<sub>/,
      /background-color:#FFF2A8/,
      /katex/,
      /Footnote body/,
      /Callout body/,
      /data-footnote-label="ref"/,
    ])
      assert.match(html, pattern);
    assert.match(html, /data-revealed="false"/);
    assert.match(html, /class="rich-hidden-value" aria-hidden="true">PRIVATE_COMMENT/);
    assert.doesNotMatch(html, /此内容格式无法显示/);
    assert.match(html, /rich-inline-atom/);
    assert.match(html, /color:#123456/);
    assert.match(html, /<strong><sup class="rich-footnote-reference"><button/);
    assert.doesNotMatch(html, /disabled=""/);
    const standalone = renderToStaticMarkup(
      createElement(StandaloneDocumentContent, {
        document: doc(
          ...document.content,
          { type: 'image', attrs: { blob_hash: 'a'.repeat(64), alt: 'Unbound image' } },
          { type: 'wikiLink', attrs: { wiki_target: '#Overview' } },
        ),
        identity,
      }),
    );
    assert.match(standalone, /katex/);
    assert.match(standalone, /Unbound image/);
    assert.doesNotMatch(standalone, /<img\b|revision=/);
    assert.match(standalone, /class="rich-hidden-value" aria-hidden="true">PRIVATE_COMMENT/);
    const card = renderToStaticMarkup(
      createElement(PreviewDocumentView, {
        document: projectPreview(document),
        title: 'Title',
        identity,
        revision: 'revision',
        imageFailed: false,
        onImageError: () => assert.fail('no image'),
      }),
    );
    assert.match(card, /<h6/);
    assert.match(card, /katex/);
    assert.doesNotMatch(card, /PRIVATE_COMMENT|<button|<dialog|<input/);
    assert.match(card, /rich-inline-atom/);
    assert.match(card, /color:#123456/);
  } finally {
    await server.close();
  }
});

test('standalone text transforms retain marks and leave code and links literal', async (context) => {
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
    const { StandaloneDocumentContent } = await server.ssrLoadModule(
      '/src/rich/DocumentContent.tsx',
    );
    const seen = [];
    const document = doc(
      {
        type: 'paragraph',
        content: [
          text('[E1]', [{ type: 'bold' }]),
          text('[E2]', [{ type: 'code' }]),
          text('[E3]', [{ type: 'link', attrs: { href: 'https://example.test' } }]),
        ],
      },
      { type: 'codeBlock', attrs: { language: 'text' }, content: [text('[E4]')] },
      { type: 'callout', attrs: { callout_type: 'note' }, content: [p('[E5]')] },
    );
    const html = renderToStaticMarkup(
      createElement(StandaloneDocumentContent, {
        document,
        identity: { local_instance_id: 'local', workspace_id: 'work' },
        renderText: (value) => {
          seen.push(value);
          return createElement('button', { type: 'button', 'data-evidence': value }, value);
        },
      }),
    );
    assert.deepEqual(seen, ['[E1]', '[E5]']);
    assert.match(html, /<strong><button type="button" data-evidence="\[E1\]"/);
    assert.match(html, /<code>\[E2\]<\/code>/);
    assert.match(html, /<a href="https:\/\/example\.test" class="document-link"[^>]*>\[E3\]<\/a>/);
    assert.match(html, /<pre[^>]*><code><span>\[E4\]<\/span><\/code><\/pre>/);
    assert.equal((html.match(/data-evidence=/g) ?? []).length, 2);
  } finally {
    await server.close();
  }
});

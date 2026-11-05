import test from 'node:test';
import assert from 'node:assert/strict';
import { Worker } from 'node:worker_threads';
import {
  appendMarkdownDocument,
  looksLikeMarkdown,
  markdownDocument,
} from '../src/inference/markdown-document.ts';
import { prepareAnswerProposal } from '../src/inference/proposal-document.ts';
import { convertSourceNote } from '../src/sources/source-note-conversion.ts';

const flatten = (nodes) => nodes.flatMap((node) => [node, ...flatten(node.content ?? [])]);
const text = (nodes) =>
  flatten(nodes)
    .filter((node) => node.type === 'text')
    .map((node) => node.text)
    .join('');
const ofType = (document, type) => flatten(document.content).filter((node) => node.type === type);
function semantic(node) {
  const result = { type: node.type };
  if (node.text != null) result.text = node.text;
  const attrs = Object.fromEntries(
    Object.entries(node.attrs ?? {}).filter(
      ([key, value]) => !['id', 'occurrence_id'].includes(key) && value != null,
    ),
  );
  if (Object.keys(attrs).length) result.attrs = attrs;
  if (node.marks?.length)
    result.marks = node.marks
      .map((mark) => ({ ...mark }))
      .sort((a, b) => a.type.localeCompare(b.type));
  if (node.content?.length) result.content = node.content.map(semantic);
  return result;
}
function parse(source, options) {
  const first = markdownDocument(source, options);
  assert.deepEqual(first.preserved, []);
  return { document: first.document };
}

test('D3 inline formats and the six OFM highlight colors retain their semantics', () => {
  const { document } = parse(
    '**Bold** *Italic* ~~strike~~ ~strike too~ <u>underlined</u> <sup>up</sup> <sub>down</sub> ' +
      '<span style="color:rgb(1, 2, 3)">color</span> ' +
      '<mark style="background-color:#abc;color:red">both</mark>\n\n' +
      '==🔴red== ==🟠orange== ==🟡yellow== ==🟢green== ==🔵blue== ==🟣purple== ==default==',
  );
  const nodes = ofType(document, 'text');
  assert.deepEqual(nodes.find((node) => node.text === 'both').marks, [
    { type: 'highlight', attrs: { color: '#AABBCC' } },
    { type: 'textStyle', attrs: { color: '#FF0000' } },
  ]);
  assert.deepEqual(
    nodes
      .filter((node) => ['red', 'orange', 'yellow', 'green', 'blue', 'purple'].includes(node.text))
      .map((node) => node.marks[0].attrs.color),
    ['#FECACA', '#FED7AA', '#FFF2A8', '#BBF7D0', '#BFDBFE', '#DDD6FE'],
  );
  assert.deepEqual(nodes.find((node) => node.text === 'default').marks, [{ type: 'highlight' }]);
});

test(
  'multiline highlight resolver terminates and leaves math delimiter content intact',
  { timeout: 5_000 },
  async () => {
    const module = new URL('../src/inference/markdown-document.ts', import.meta.url).href;
    const worker = new Worker(
      new URL(
        `data:text/javascript,${encodeURIComponent(
          `import {parentPort} from 'node:worker_threads'; import {markdownDocument} from ${JSON.stringify(module)}; parentPort.postMessage(markdownDocument('==first\\nsecond $a==b$=='));`,
        )}`,
      ),
      { execArgv: ['--experimental-strip-types'] },
    );
    const result = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        void worker.terminate();
        reject(new Error('Markdown parsing did not finish.'));
      }, 3_000);
      worker.once('message', (value) => {
        clearTimeout(timer);
        resolve(value);
      });
      worker.once('error', (error) => {
        clearTimeout(timer);
        reject(error);
      });
    });
    await worker.terminate();
    assert.deepEqual(result.preserved, []);
    assert.equal(result.document.content[0].content[0].text, 'first\nsecond ');
    assert.equal(ofType(result.document, 'inlineMath')[0].attrs.source, 'a==b');
    assert.deepEqual(ofType(result.document, 'inlineMath')[0].marks, [{ type: 'highlight' }]);
  },
);

test('fences, inline code and escapes protect literal OFM and unknown code combinations', () => {
  const result = markdownDocument(
    String.raw`\==literal== \[[note]] \%%visible%% \^[note]` +
      '\n\n' +
      '`==not mark== [[note]] %%raw%% $x$`\n\n```md\n==no==\n[[note]]\n%%raw%%\n---\na: b\n---\n```',
  );
  assert.deepEqual(result.preserved, []);
  assert.equal(ofType(result.document, 'wikiLink').length, 0);
  assert.equal(ofType(result.document, 'comment').length, 0);
  assert.equal(ofType(result.document, 'frontmatter').length, 0);
  assert.ok(ofType(result.document, 'text').some((node) => node.marks?.[0]?.type === 'code'));
  assert.equal(
    ofType(result.document, 'codeBlock')[0].content[0].text,
    '==no==\n[[note]]\n%%raw%%\n---\na: b\n---',
  );
  const conflict = markdownDocument('==text `code`==');
  assert.deepEqual(conflict.preserved, ['code']);
  assert.equal(text(conflict.document.content), '==text `code`==');
});

test('wiki targets, relative Markdown links, aliases, embeds and block anchors are distinct', () => {
  const { document } = parse(
    '###### Deep heading ^head\n\nA block. ^paragraph\n\n' +
      '[[Note#Heading#Child|Alias]] [[#^paragraph]] [Heading](#head) [Other](folder/Note.md#Title)\n\n' +
      '![[photo.png|alt text|200x100]]\n\n> quote\n\n^quote-id',
  );
  assert.equal(document.content[0].attrs.level, 6);
  assert.equal(document.content[0].attrs.block_id, 'head');
  assert.equal(document.content[1].attrs.block_id, 'paragraph');
  const links = ofType(document, 'wikiLink');
  assert.deepEqual(
    links.map((node) => node.attrs.wiki_target),
    ['Note#Heading#Child', '#^paragraph', '#head', 'folder/Note.md#Title'],
  );
  assert.ok(links.every((node) => !node.attrs.target_id));
  const embed = ofType(document, 'wikiEmbed')[0];
  assert.equal(embed.attrs.embed_width, 200);
  assert.equal(embed.attrs.embed_height, 100);
  assert.equal(embed.attrs.label, 'alt text');
  assert.equal(ofType(document, 'blockquote')[0].attrs.block_id, 'quote-id');
  const escaped = markdownDocument(String.raw`Literal \^not-an-anchor`);
  assert.equal(escaped.document.content[0].attrs.block_id, undefined);
});

test('callout aliases, case, folding, nesting and plain-text titles parse safely', () => {
  const { document } = parse(
    '> [!FAQ]- Question\n> body\n>\n> > [!CUSTOM_kind]+ Nested\n> > content\n\n' +
      '> [!note] <script>alert(1)</script> &amp; **literal title**\n\n> [!INFO]',
  );
  const callouts = ofType(document, 'callout');
  assert.deepEqual(
    callouts.map((node) => node.attrs.callout_type),
    ['FAQ', 'CUSTOM_kind', 'note', 'INFO'],
  );
  assert.deepEqual(
    callouts.map((node) => node.attrs.collapsed),
    [true, false, undefined, undefined],
  );
  assert.equal(callouts[2].attrs.title, '<script>alert(1)</script> & **literal title**');
  assert.equal(callouts[3].content.length, 0);
});

test('frontmatter retains YAML source and append contexts keep it visibly literal', () => {
  const yaml =
    'title: Example\naliases: [one, two]\nunknown:\n  nested: null\nquoted: "[[Note]]"\n';
  const source = `---\n${yaml}\n---\n\nBody`;
  const { document } = parse(source);
  assert.equal(document.content[0].attrs.source, yaml);
  const base = markdownDocument('Existing body').document;
  const result = appendMarkdownDocument(base, source);
  assert.ok(result.preserved.includes('frontmatter'));
  assert.equal(ofType(result.document, 'frontmatter').length, 0);
  assert.ok(text(result.document.content).includes(`---\n${yaml}\n---`));
  assert.equal(base.content.length, 1);
  assert.equal(base.content[0].content[0].text, 'Existing body');
  const forced = appendMarkdownDocument(base, source, { allowFrontmatter: true });
  assert.ok(forced.preserved.includes('frontmatter'));
  assert.equal(ofType(forced.document, 'frontmatter').length, 0);
});

test('named, missing and inline footnotes keep content and disambiguate pasted labels', () => {
  const { document } = parse(
    'A[^Ref]^[inline **bold** and [link](https://example.test)] [^missing]\n\n[^ref]: First\n\n    Second',
  );
  assert.equal(ofType(document, 'footnoteDefinition').length, 2);
  assert.equal(ofType(document, 'footnoteDefinition')[0].content.length, 2);
  assert.equal(ofType(document, 'footnoteReference').at(-1).attrs.label, 'missing');
  const copied = markdownDocument('New[^REF]\n\n[^ref]: New definition', {
    contextDocument: document,
  });
  const definition = ofType(copied.document, 'footnoteDefinition')[0];
  assert.equal(definition.attrs.label, 'ref-2');
  assert.equal(ofType(copied.document, 'footnoteReference')[0].attrs.label, 'ref-2');
  const duplicate = markdownDocument('[^one]: first\n\n[^ONE]: second');
  assert.ok(duplicate.preserved.includes('footnote'));
  assert.ok(text(duplicate.document.content).includes('[^ONE]: second'));
});

test('footnote identity follows core Unicode whitespace and case rules without compatibility folding', () => {
  const { document } = parse('Circle[^①] Digit[^1]\n\n[^①]: Circle\n\n[^1]: Digit');
  assert.deepEqual(
    ofType(document, 'footnoteDefinition').map((node) => node.attrs.label),
    ['①', '1'],
  );
  const base = {
    type: 'doc',
    content: [
      {
        type: 'footnoteDefinition',
        attrs: { id: 'existing', label: 'Alpha\u2003  BETA' },
        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Existing' }] }],
      },
    ],
  };
  const appended = appendMarkdownDocument(
    base,
    'New[^ALPHA&#32;BETA]\n\n[^alpha&#32;beta]: New definition',
  );
  assert.deepEqual(appended.preserved, []);
  assert.equal(ofType(appended.document, 'footnoteDefinition')[1].attrs.label, 'alpha beta-2');
  assert.equal(ofType(appended.document, 'footnoteReference')[0].attrs.label, 'alpha beta-2');
  assert.equal(base.content[0].attrs.label, 'Alpha\u2003  BETA');
});

test('inline and standalone comments retain uninterpreted source, including line breaks', () => {
  const source =
    'Before %%hidden **raw**\nsecond line%% after\n\n%%\nblock\n\n<script>source</script>\n%%';
  const { document } = parse(source);
  assert.equal(ofType(document, 'comment')[0].attrs.source, 'hidden **raw**\nsecond line');
  assert.equal(
    ofType(document, 'commentBlock')[0].attrs.source,
    '\nblock\n\n<script>source</script>\n',
  );
  assert.equal(text(document.content), 'Before  after');
  const mixed = markdownDocument('Before %%open\n\n**middle** close%% after');
  assert.ok(mixed.preserved.includes('unsupported'));
  assert.ok(text(mixed.document.content).includes('**middle**'));
  assert.equal(
    ofType(mixed.document, 'text').some((node) => node.marks?.length),
    false,
  );
});

test(
  'standalone multiline comment tokenization terminates and retains exact source boundaries',
  { timeout: 5_000 },
  async () => {
    const module = new URL('../src/inference/markdown-document.ts', import.meta.url).href;
    const sources = [
      'single',
      'line1\nline2',
      'line1\r\nline2',
      '\n',
      '\nblock\n\n<script>literal</script>\n',
      'slash\\\nend',
    ];
    const worker = new Worker(
      new URL(
        `data:text/javascript,${encodeURIComponent(
          `import {parentPort} from 'node:worker_threads'; import {markdownDocument} from ${JSON.stringify(module)}; parentPort.postMessage(${JSON.stringify(sources)}.map(source => markdownDocument('%%' + source + '%%')));`,
        )}`,
      ),
      { execArgv: ['--experimental-strip-types'] },
    );
    const results = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        void worker.terminate();
        reject(new Error('Multiline comments did not finish parsing.'));
      }, 3_000);
      worker.once('message', (value) => {
        clearTimeout(timer);
        resolve(value);
      });
      worker.once('error', (error) => {
        clearTimeout(timer);
        reject(error);
      });
    });
    await worker.terminate();
    for (const [index, result] of results.entries()) {
      assert.deepEqual(result.preserved, []);
      assert.equal(result.document.content.length, 1);
      assert.equal(result.document.content[0].type, 'commentBlock');
      assert.equal(result.document.content[0].attrs.source, sources[index]);
    }
  },
);

test('math and Mermaid keep source, including literal dollar delimiters and code fences', () => {
  const source = '$a **not bold**$\n\n$$\n\\frac{a}{b}\n$$\n\n```mermaid\ngraph TD\nA-->B\n```';
  const { document } = parse(source);
  assert.equal(ofType(document, 'inlineMath')[0].attrs.source, 'a **not bold**');
  assert.equal(ofType(document, 'blockMath')[0].attrs.source, '\\frac{a}{b}');
});

test('GFM alignment and OFM custom task states preserve order without interpreting escaped task syntax', () => {
  const { document } = parse(
    '| left | middle | right |\n| :--- | :---: | ---: |\n| [[Note\\|Alias]] | two | three |\n\n' +
      '- [/] progress\n- [?] question\n- [😀] done\n- [ ] open\n- [x] checked\n\n- \\[/] literal',
  );
  const row = ofType(document, 'tableRow')[0];
  assert.deepEqual(
    row.content.map((cell) => cell.attrs.align),
    ['left', 'center', 'right'],
  );
  const tasks = ofType(document, 'taskItem');
  assert.deepEqual(
    tasks.map((item) => item.attrs.checked),
    [true, true, true, false, true],
  );
  assert.deepEqual(
    tasks.slice(0, 3).map((item) => item.attrs.task_marker),
    ['/', '?', '😀'],
  );
  assert.equal(ofType(document, 'listItem').at(-1).content[0].content[0].text, '[/] literal');
});

test('HTML subset rejects executable, malformed and conflicting formatting without losing the source', () => {
  const samples = [
    '<script>alert(1)</script>',
    '<span style="color:red" onclick="alert(1)">text</span>',
    '<span style="color:red;background-image:url(https://example.test/private)">text</span>',
    '<mark style="color:var(--unknown)">text</mark>',
    '<span style="color:red" style="color:blue">text</span>',
    '<sup><sub>conflict</sub></sup>',
    '<u>unclosed',
    '<a href="javascript:alert(1)">text</a>',
    '<img src=x onerror=alert(1)>',
  ];
  for (const source of samples) {
    const result = markdownDocument(source);
    assert.ok(result.preserved.includes('html'), source);
    assert.equal(text(result.document.content), source);
    assert.equal(
      flatten(result.document.content).some((node) => node.marks?.length),
      false,
    );
  }
});

test('safe link titles, targets and inert relations survive Markdown source conversion', () => {
  const { document } = parse(
    '<a href="https://example.test/?a=1&amp;b=2" title="A &quot;quoted&quot; title" target="_blank" rel="noopener noreferrer nofollow" class="">Link</a> ' +
      '<a href="mailto:person@example.test" target="_self" title="">Mail</a>',
  );
  const links = ofType(document, 'text').filter((node) =>
    node.marks?.some((mark) => mark.type === 'link'),
  );
  assert.deepEqual(links[0].marks[0].attrs, {
    href: 'https://example.test/?a=1&b=2',
    title: 'A "quoted" title',
    target: '_blank',
    rel: 'noopener noreferrer nofollow',
    class: '',
  });
  assert.equal(links[1].marks[0].attrs.target, '_self');
  assert.equal(links[1].marks[0].attrs.title, '');
});

test('bounds and anchor conflicts remain explicit and leave the input source visible', () => {
  const base = markdownDocument('Existing ^same').document;
  const conflicting = markdownDocument('Pasted ^same', { contextDocument: base });
  assert.deepEqual(conflicting.preserved, ['anchor']);
  assert.equal(text(conflicting.document.content), 'Pasted ^same');
  assert.equal(markdownDocument('^alone').document.content[0].attrs.block_id, undefined);
  const longMath = `$${'中'.repeat(22_000)}$`;
  const oversized = markdownDocument(longMath);
  assert.ok(oversized.preserved.includes('bounds'));
  assert.equal(text(oversized.document.content), longMath);
  for (const source of ['![[image|4097]]', `[[${'a'.repeat(2049)}]]`]) {
    const result = markdownDocument(source);
    assert.ok(result.preserved.includes('wiki'));
    assert.equal(text(result.document.content), source);
  }
});

test('Markdown source boundaries reject NUL and excessive footnotes before producing invalid D3', () => {
  for (const source of ['plain\0text', '$x\0$', '%%hidden\0%%', '---\nproperty: a\0b\n---'])
    assert.throws(() => markdownDocument(source), /NUL/);
  const title = '> [!note] title\twith a tab';
  const result = markdownDocument(title);
  assert.ok(result.preserved.includes('bounds'));
  assert.equal(text(result.document.content), title);
  const definitions = Array.from(
    { length: 4097 },
    (_, index) => `[^note-${index}]: definition`,
  ).join('\n\n');
  assert.throws(() => markdownDocument(definitions), /too many footnotes/);
});

test('source imports and answer proposals share rich semantics while preserving the fixed evidence tail', () => {
  const markdown = '> [!tip] Tip\n> ==highlight== and $x$\n\n[[Note]]';
  const imported = convertSourceNote(`\uFEFF${markdown.replace(/\n/g, '\r\n')}`, 'markdown');
  const base = markdownDocument('Existing **bold**').document;
  const original = structuredClone(base);
  const prepared = prepareAnswerProposal(base, {
    request: { search: { query: 'Question' } },
    text: markdown,
    evidence: [
      {
        citation_id: 'E1',
        evidence: {
          title: 'Evidence',
          target: {
            object_id: 'note-id',
            content_revision: 'fixed-revision',
            block_id: 'fixed-block',
          },
        },
      },
    ],
  });
  assert.deepEqual(prepared.preserved, []);
  assert.deepEqual(base, original);
  assert.deepEqual(
    semantic({ type: 'doc', content: prepared.document.content.slice(2, -1) }),
    semantic(imported.document),
  );
  const reference = ofType(prepared.document, 'noteReference')[0];
  assert.equal(reference.attrs.target_revision_id, 'fixed-revision');
  assert.equal(reference.attrs.target_block_id, 'fixed-block');
});

test('Markdown paste detection includes OFM, single emphasis and tables without interpreting plain text', () => {
  for (const source of [
    '*italic*',
    '[[Note]]',
    '==highlight==',
    '$x$',
    '%%hidden%%',
    '<u>u</u>',
    '| a |\n| --- |\n| b |',
  ])
    assert.equal(looksLikeMarkdown(source), true, source);
  assert.equal(looksLikeMarkdown('普通文字 plain text'), false);
});

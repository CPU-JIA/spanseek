import assert from 'node:assert/strict';
import test from 'node:test';
import { Editor, getExtensionField } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import UniqueID from '@tiptap/extension-unique-id';
import { Table, TableRow } from '@tiptap/extension-table';
import TaskList from '@tiptap/extension-task-list';
import {
  AlignedTableCell,
  AlignedTableHeader,
  RichBlockAttributes,
  RichOrderedList,
  RichTaskItem,
  richSchemaExtensions,
} from '../src/editor/rich-schema.ts';
import {
  RICH_BLOCK_TYPES,
  blockAnchorError,
  captureRichEditTarget,
  dimensionValue,
  editorColor,
  findNodePosition,
  footnoteIndex,
  footnoteLabelError,
  insertFootnote,
  mapRichEditTarget,
  renameFootnoteDefinition,
  richDocumentError,
  richEditTargetPosition,
  selectedRichEdit,
  sourceError,
  validTaskMarker,
  wikiTargetError,
} from '../src/editor/rich-format-model.ts';
import { insertEditorMarkdown, richMarkdownPaste } from '../src/editor/rich-markdown.ts';
import { MIRROR, mirrorTransactions } from '../src/editor/mirror-transactions.ts';

const paragraph = (id, text = '', attrs = {}) => ({
  type: 'paragraph',
  attrs: { id, ...attrs },
  ...(text ? { content: [{ type: 'text', text }] } : {}),
});
function createEditor(content = [paragraph('original')], notices = []) {
  const editor = new Editor({
    element: null,
    enablePasteRules: false,
    extensions: [
      StarterKit.configure({
        heading: { levels: [1, 2, 3, 4, 5, 6] },
        orderedList: false,
        trailingNode: false,
      }),
      UniqueID.configure({
        attributeName: 'id',
        types: RICH_BLOCK_TYPES,
        generateID: () => crypto.randomUUID(),
        filterTransaction: (transaction) => !transaction.getMeta(MIRROR),
      }),
      Table,
      TableRow,
      AlignedTableCell,
      AlignedTableHeader,
      RichOrderedList,
      TaskList,
      RichTaskItem.configure({ nested: true }),
      ...richSchemaExtensions.filter((extension) => extension.name !== 'richBlockAttributes'),
      RichBlockAttributes.configure({ onError: (message) => notices.push(message) }),
    ],
    content: { type: 'doc', content },
  });
  // Exercise the real transaction and input-rule plugins without pretending to
  // test a browser's DOM, selection, clipboard, or input-method composition.
  editor.view.updateState(editor.state.reconfigure({ plugins: editor.extensionManager.plugins }));
  return editor;
}
function input(editor, source) {
  for (const text of source) {
    const { from, to } = editor.state.selection;
    const handled = editor.state.plugins.some((plugin) =>
      plugin.props.handleTextInput?.(editor.view, from, to, text, () => editor.state.tr),
    );
    if (!handled) editor.view.dispatch(editor.state.tr.insertText(text, from, to));
  }
}
function inputEnter(editor) {
  return editor.state.plugins.some(
    (plugin) =>
      plugin.spec.isInputRules && plugin.props.handleKeyDown?.(editor.view, { key: 'Enter' }),
  );
}
function nodes(editor, type) {
  const result = [];
  editor.state.doc.descendants((node, position) => {
    if (node.type.name === type) result.push({ node, position });
  });
  return result;
}

test('ordered lists preserve numbering and block identities without exporting unsupported marker types', () => {
  const editor = createEditor([
    {
      type: 'orderedList',
      attrs: { id: 'list', start: 7, type: null },
      content: [
        {
          type: 'listItem',
          attrs: { id: 'item' },
          content: [paragraph('body', '保留正文')],
        },
      ],
    },
    paragraph('after', '后文'),
  ]);
  try {
    const list = editor.getJSON().content[0];
    assert.deepEqual({ ...list.attrs }, { id: 'list', block_id: null, start: 7 });
    assert.equal(list.content[0].attrs.id, 'item');
    assert.equal(list.content[0].content[0].attrs.id, 'body');
    assert.equal(list.content[0].content[0].content[0].text, '保留正文');
    assert.equal(editor.state.doc.lastChild.attrs.id, 'after');
    const paragraphPosition = nodes(editor, 'paragraph')[0].position;
    editor.commands.setTextSelection(paragraphPosition + 1);
    assert.equal(editor.commands.toggleOrderedList(), true);
    assert.equal(editor.state.doc.firstChild.type.name, 'paragraph');
    assert.equal(editor.state.doc.firstChild.attrs.id, 'body');
    assert.equal(editor.commands.toggleOrderedList(), true);
    assert.equal(editor.state.doc.firstChild.type.name, 'orderedList');
    assert.equal(Object.hasOwn(editor.getJSON().content[0].attrs, 'type'), false);
  } finally {
    editor.destroy();
  }
  const typed = createEditor();
  try {
    input(typed, '3. ');
    assert.equal(typed.state.doc.firstChild.type.name, 'orderedList');
    assert.equal(typed.state.doc.firstChild.attrs.start, 3);
    assert.equal(Object.hasOwn(typed.getJSON().content[0].attrs, 'type'), false);
    assert.equal(nodes(typed, 'paragraph')[0].node.attrs.id, 'original');
  } finally {
    typed.destroy();
  }
});

test('format fields enforce UTF-8 bytes and Unicode character limits without truncating source', () => {
  assert.equal(sourceError('😀'.repeat(16384)), null);
  assert.match(sourceError('😀'.repeat(16384) + 'a'), /64 KiB/);
  assert.equal(sourceError(''), null);
  assert.match(sourceError('a\u0000b'), /U\+0000/);
  assert.ok(sourceError(' \n', true));
  assert.equal(footnoteLabelError('😀'.repeat(512)), null);
  assert.ok(footnoteLabelError('😀'.repeat(513)));
  assert.ok(footnoteLabelError('a\u0085'));
  assert.ok(wikiTargetError('note\u009f'));
  assert.equal(wikiTargetError('资料/笔记#^anchor'), null);
  assert.equal(validTaskMarker('😀'), true);
  assert.equal(validTaskMarker('/'), true);
  assert.equal(validTaskMarker(' '), true);
  for (const marker of ['', 'ab', '[]', '\n', '\u0085'])
    assert.equal(validTaskMarker(marker), false);
  assert.equal(dimensionValue(''), null);
  assert.equal(dimensionValue('4096'), 4096);
  assert.throws(() => dimensionValue('1.5'));
  assert.throws(() => dimensionValue('4097'));
  assert.equal(editorColor('#AbC'), '#AABBCC');
  assert.equal(editorColor('rgb(1, 127, 255)'), '#017FFF');
  assert.equal(editorColor('rgba(1,2,3,0.1)'), null);
  assert.equal(editorColor('red; background:url(x)'), null);
});

test('real formatting commands keep block identities and enforce code and script mark exclusions', () => {
  const editor = createEditor([paragraph('first', '格式正文'), paragraph('untouched', '后文')]);
  try {
    editor.commands.setTextSelection({ from: 1, to: 5 });
    editor
      .chain()
      .toggleUnderline()
      .toggleStrike()
      .setMark('highlight', { color: '#BFDBFE' })
      .setMark('textStyle', { color: '#B91C1C' })
      .setMark('superscript')
      .run();
    let marks = editor.state.doc.firstChild.firstChild.marks;
    assert.deepEqual(
      new Set(marks.map((mark) => mark.type.name)),
      new Set(['underline', 'strike', 'highlight', 'textStyle', 'superscript']),
    );
    assert.ok(
      marks.findIndex((mark) => mark.type.name === 'highlight') <
        marks.findIndex((mark) => mark.type.name === 'textStyle'),
    );
    editor.commands.setMark('subscript');
    marks = editor.state.doc.firstChild.firstChild.marks;
    assert.equal(
      marks.some((mark) => mark.type.name === 'superscript'),
      false,
    );
    assert.equal(
      marks.some((mark) => mark.type.name === 'subscript'),
      true,
    );
    editor.commands.toggleCode();
    assert.deepEqual(
      editor.state.doc.firstChild.firstChild.marks.map((mark) => mark.type.name),
      ['code'],
    );
    editor.commands.toggleCode();
    for (let level = 1; level <= 6; ++level) {
      assert.equal(editor.commands.setHeading({ level }), true);
      assert.equal(editor.state.doc.firstChild.attrs.level, level);
      assert.equal(editor.state.doc.firstChild.attrs.id, 'first');
    }
    assert.equal(editor.state.doc.lastChild.attrs.id, 'untouched');
    assert.equal(editor.state.doc.lastChild.textContent, '后文');
  } finally {
    editor.destroy();
  }
});

test('rich nodes and table alignment survive a schema round trip with unchanged nested IDs', () => {
  const editor = createEditor([
    { type: 'frontmatter', attrs: { id: 'yaml', source: 'title: 笔记\n# 原注释' } },
    {
      type: 'callout',
      attrs: { id: 'callout', callout_type: 'custom-TIP', collapsed: true, title: '折叠标题' },
      content: [
        {
          type: 'heading',
          attrs: { id: 'h6', level: 6 },
          content: [
            { type: 'inlineMath', attrs: { source: 'E=mc^2' }, marks: [{ type: 'underline' }] },
            {
              type: 'wikiLink',
              attrs: { wiki_target: '笔记#^p1', label: '别名', occurrence_id: 'wiki-occurrence' },
            },
            { type: 'comment', attrs: { source: '隐藏的注释' } },
          ],
        },
      ],
    },
    { type: 'blockMath', attrs: { id: 'math', source: '\\sum_{i=0}^{n} i', block_id: 'sum' } },
    {
      type: 'wikiEmbed',
      attrs: {
        id: 'embed',
        wiki_target: '图片.png',
        occurrence_id: 'embed-occurrence',
        embed_width: 320,
        embed_height: 240,
      },
    },
    { type: 'commentBlock', attrs: { id: 'comment', source: '多行\n注释' } },
    {
      type: 'table',
      attrs: { id: 'table' },
      content: [
        {
          type: 'tableRow',
          attrs: { id: 'row' },
          content: [
            {
              type: 'tableHeader',
              attrs: { id: 'header', align: 'left' },
              content: [paragraph('header-text', '表头')],
            },
            {
              type: 'tableCell',
              attrs: { id: 'cell', align: 'right' },
              content: [paragraph('cell-text', '数值')],
            },
          ],
        },
      ],
    },
  ]);
  try {
    const cell = nodes(editor, 'tableCell')[0];
    editor.commands.setTextSelection(cell.position + 2);
    assert.equal(editor.commands.setCellAttribute('align', 'center'), true);
    const original = editor.getJSON();
    assert.deepEqual(editor.schema.nodeFromJSON(original).toJSON(), original);
    assert.equal(nodes(editor, 'tableHeader')[0].node.attrs.align, 'left');
    assert.equal(nodes(editor, 'tableCell')[0].node.attrs.align, 'center');
    assert.equal(nodes(editor, 'tableCell')[0].node.attrs.id, 'cell');
    assert.equal(nodes(editor, 'inlineMath')[0].node.marks[0].type.name, 'underline');
    assert.equal(richDocumentError(editor.state.doc), null);
  } finally {
    editor.destroy();
  }
});

test('footnote insertion and rename are undoable transactions and retain original block IDs', () => {
  const editor = createEditor([paragraph('first', '正文'), paragraph('last', '保留')]);
  try {
    editor.commands.setTextSelection(3);
    editor.view.dispatch(insertFootnote(editor.state, '甲', '脚注正文'));
    assert.equal(nodes(editor, 'footnoteReference').length, 1);
    assert.equal(editor.state.doc.firstChild.attrs.id, 'first');
    assert.equal(findNodePosition(editor.state.doc, 'last'), 5);
    const definition = nodes(editor, 'footnoteDefinition')[0];
    const definitionId = definition.node.attrs.id;
    editor.view.dispatch(renameFootnoteDefinition(editor.state, definition.position, '乙'));
    assert.equal(nodes(editor, 'footnoteDefinition')[0].node.attrs.id, definitionId);
    assert.equal(nodes(editor, 'footnoteReference')[0].node.attrs.label, '乙');
    assert.equal(footnoteIndex(editor.state.doc).order.get('乙'), 1);
    assert.throws(() => insertFootnote(editor.state, '乙', '重复'), /已有定义/);
    assert.equal(editor.commands.undo(), true);
    assert.equal(editor.commands.redo(), true);
    assert.equal(nodes(editor, 'footnoteReference')[0].node.attrs.label, '乙');
  } finally {
    editor.destroy();
  }
});

test('footnote rename rejects collisions before changing the definition or its references', () => {
  const editor = createEditor([
    {
      type: 'paragraph',
      attrs: { id: 'p' },
      content: [{ type: 'footnoteReference', attrs: { label: 'one' } }],
    },
    {
      type: 'footnoteDefinition',
      attrs: { id: 'd1', label: 'one' },
      content: [paragraph('b1', '一')],
    },
    {
      type: 'footnoteDefinition',
      attrs: { id: 'd2', label: 'two' },
      content: [paragraph('b2', '二')],
    },
  ]);
  try {
    const before = editor.getJSON();
    assert.throws(
      () =>
        renameFootnoteDefinition(
          editor.state,
          nodes(editor, 'footnoteDefinition')[0].position,
          'two',
        ),
      /已有定义/,
    );
    assert.deepEqual(editor.getJSON(), before);
  } finally {
    editor.destroy();
  }
});

test('definition typing is not intercepted by the shorter footnote-reference rule', () => {
  const editor = createEditor();
  try {
    input(editor, '[^中文脚注]');
    assert.equal(nodes(editor, 'footnoteReference').length, 0);
    input(editor, ': ');
    assert.equal(nodes(editor, 'footnoteDefinition').length, 1);
    assert.equal(nodes(editor, 'footnoteDefinition')[0].node.attrs.label, '中文脚注');
    assert.equal(nodes(editor, 'paragraph')[0].node.attrs.id, 'original');
    input(editor, '注释正文');
    assert.equal(nodes(editor, 'footnoteDefinition')[0].node.textContent, '注释正文');
  } finally {
    editor.destroy();
  }
  const reference = createEditor();
  try {
    input(reference, '正文[^中文脚注] ');
    assert.equal(nodes(reference, 'footnoteReference')[0].node.attrs.label, '中文脚注');
    assert.equal(reference.state.doc.firstChild.lastChild.text, ' ');
  } finally {
    reference.destroy();
  }
});

for (const [source, type, field, expected] of [
  ['==🔴重点==', 'text', 'highlight', '#FECACA'],
  ['$x^2$', 'inlineMath', 'source', 'x^2'],
  ['$$x+y$$', 'blockMath', 'source', 'x+y'],
  ['[[笔记#标题|别名]]', 'wikiLink', 'wiki_target', '笔记#标题'],
  ['![[图像.png|320x240]]', 'wikiEmbed', 'embed_width', 320],
  ['%%内部注释%%', 'comment', 'source', '内部注释'],
]) {
  test('input rule converts ' + source + ' without replacing unrelated blocks', () => {
    const editor = createEditor([paragraph('original'), paragraph('untouched', '保留原文')]);
    try {
      input(editor, source);
      const converted = nodes(editor, type)[0].node;
      assert.equal(
        field === 'highlight'
          ? converted.marks.find((mark) => mark.type.name === 'highlight')?.attrs.color
          : converted.attrs[field],
        expected,
      );
      assert.equal(editor.state.doc.firstChild.attrs.id, 'original');
      assert.equal(editor.state.doc.lastChild.attrs.id, 'untouched');
      assert.equal(editor.state.doc.lastChild.textContent, '保留原文');
      assert.equal(richDocumentError(editor.state.doc), null);
    } finally {
      editor.destroy();
    }
  });
}

test('callout headers retain a typed title and fold state when Enter commits the header', () => {
  const editor = createEditor();
  try {
    input(editor, '> [!warning]- 警告标题');
    assert.equal(nodes(editor, 'callout').length, 0);
    assert.equal(inputEnter(editor), true);
    const callout = nodes(editor, 'callout')[0].node;
    assert.equal(callout.attrs.callout_type, 'warning');
    assert.equal(callout.attrs.title, '警告标题');
    assert.equal(callout.attrs.collapsed, true);
    assert.equal(callout.firstChild.attrs.id, 'original');
    input(editor, '正文');
    assert.equal(nodes(editor, 'callout')[0].node.textContent, '正文');
  } finally {
    editor.destroy();
  }
});

test('custom task markers are checked, clear when toggled, and do not leak into a new task', () => {
  const editor = createEditor();
  try {
    input(editor, '[😀] 内容');
    const first = nodes(editor, 'taskItem')[0];
    assert.equal(first.node.attrs.checked, true);
    assert.equal(first.node.attrs.task_marker, '😀');
    assert.equal(editor.commands.splitListItem('taskItem'), true);
    const tasks = nodes(editor, 'taskItem');
    assert.equal(tasks.length, 2);
    assert.equal(tasks[0].node.attrs.task_marker, '😀');
    assert.equal(tasks[1].node.attrs.checked, false);
    assert.equal(tasks[1].node.attrs.task_marker, null);
    editor.view.dispatch(
      editor.state.tr.setNodeMarkup(tasks[0].position, undefined, {
        ...tasks[0].node.attrs,
        checked: false,
      }),
    );
    assert.equal(nodes(editor, 'taskItem')[0].node.attrs.task_marker, null);
  } finally {
    editor.destroy();
  }
});

test('document guard rejects duplicate anchors and misplaced YAML while preserving the old document', () => {
  const notices = [];
  const editor = createEditor(
    [paragraph('first', '正文', { block_id: 'stable-anchor' }), paragraph('other', '保留')],
    notices,
  );
  try {
    const before = editor.getJSON();
    editor.commands.insertContentAt(
      editor.state.doc.content.size,
      paragraph('copied', '重复', { block_id: 'stable-anchor' }),
    );
    assert.deepEqual(editor.getJSON(), before);
    assert.match(notices.at(-1), /重复块标识/);
    editor.commands.insertContentAt(editor.state.doc.content.size, {
      type: 'frontmatter',
      attrs: { id: 'bad-yaml', source: 'a: b' },
    });
    assert.deepEqual(editor.getJSON(), before);
    assert.match(notices.at(-1), /文档顶部/);
    assert.equal(blockAnchorError('stable-anchor', editor.state.doc, 'first'), null);
    assert.ok(blockAnchorError('stable-anchor', editor.state.doc, 'other'));
    editor.commands.setTextSelection(3);
    editor.commands.splitBlock();
    assert.equal(editor.state.doc.firstChild.attrs.block_id, 'stable-anchor');
    assert.equal(editor.state.doc.child(1).attrs.block_id, null);
  } finally {
    editor.destroy();
  }
});

test('Markdown insertion changes only the selection and mirrors across independent schemas', () => {
  const content = [paragraph('first', '前后'), paragraph('untouched', '尾段')];
  const left = createEditor(content),
    right = createEditor(content);
  const notices = [];
  const transactions = [];
  try {
    left.commands.setTextSelection(2);
    left.on('transaction', ({ transaction, appendedTransactions }) => {
      transactions.push(transaction, ...appendedTransactions);
    });
    assert.equal(
      insertEditorMarkdown(left, '**新增** 和 $x$\n\n> [!tip]+ 标题\n> 提示正文', (notice) =>
        notices.push(notice),
      ),
      true,
    );
    right.view.dispatch(mirrorTransactions(right.state, transactions));
    assert.deepEqual(right.getJSON(), left.getJSON());
    assert.equal(left.state.doc.lastChild.attrs.id, 'untouched');
    assert.equal(left.state.doc.lastChild.textContent, '尾段');
    assert.equal(nodes(left, 'callout')[0].node.attrs.title, '标题');
    assert.equal(nodes(left, 'inlineMath')[0].node.attrs.source, 'x');
    assert.ok(left.state.doc.textContent.includes('前'));
    assert.ok(left.state.doc.textContent.includes('后'));
    assert.equal(notices.at(-1), null);
    // A selection-local replacement cannot erase the trailing stable paragraph.
    const firstChange = transactions
      .find((transaction) => transaction.docChanged)
      .steps[0].toJSON();
    assert.equal(firstChange.from, 2);
    assert.equal(firstChange.to, 2);
  } finally {
    left.destroy();
    right.destroy();
  }
});

test('format dialog targets follow preceding edits and reject deletion beside an identical atom', () => {
  const editor = createEditor([
    {
      type: 'paragraph',
      attrs: { id: 'p' },
      content: [
        { type: 'inlineMath', attrs: { source: 'x' } },
        { type: 'inlineMath', attrs: { source: 'x' } },
      ],
    },
  ]);
  try {
    let target = captureRichEditTarget(editor.state, 1);
    const before = editor.state.tr.insertText('前文', 1);
    target = mapRichEditTarget(target, before);
    editor.view.dispatch(before);
    assert.equal(richEditTargetPosition(editor.state.doc, target), 3);
    const deletion = editor.state.tr.delete(3, 4);
    target = mapRichEditTarget(target, deletion);
    editor.view.dispatch(deletion);
    assert.equal(nodes(editor, 'inlineMath').length, 1);
    assert.throws(() => richEditTargetPosition(editor.state.doc, target), /已在编辑期间改变/);
    assert.throws(
      () => richEditTargetPosition(editor.state.doc, captureRichEditTarget(editor.state, 1000)),
      /已在编辑期间改变/,
    );
  } finally {
    editor.destroy();
  }
});

test('selected format actions carry the current atom kind and position across selection-only changes', () => {
  const editor = createEditor([
    {
      type: 'paragraph',
      attrs: { id: 'p' },
      content: [
        { type: 'inlineMath', attrs: { source: 'x' } },
        { type: 'comment', attrs: { source: '注释' } },
      ],
    },
  ]);
  try {
    assert.equal(selectedRichEdit(editor.state), null);
    editor.commands.setNodeSelection(1);
    assert.deepEqual(selectedRichEdit(editor.state), { kind: 'inlineMath', position: 1 });
    editor.commands.setNodeSelection(2);
    assert.deepEqual(selectedRichEdit(editor.state), { kind: 'comment', position: 2 });
    editor.commands.setTextSelection(3);
    assert.equal(selectedRichEdit(editor.state), null);
  } finally {
    editor.destroy();
  }
});

test('HTML attribute rules keep numeric and boolean-looking labels and callout titles as strings', () => {
  const editor = createEditor();
  try {
    for (const type of [
      'footnoteReference',
      'footnoteDefinition',
      'wikiLink',
      'wikiEmbed',
      'callout',
    ]) {
      for (const value of ['1', '001', 'true', 'false']) {
        const attribute = type === 'callout' ? 'title' : 'label';
        const parse = editor.schema.nodes[type].spec.parseDOM[0];
        const attributes = parse.getAttrs({
          getAttribute: (name) =>
            name === attribute ? value : name === 'data-wiki-target' ? '目标' : null,
        });
        assert.equal(attributes[attribute], value, type + ': ' + value);
        assert.equal(typeof attributes[attribute], 'string');
      }
    }
  } finally {
    editor.destroy();
  }
});

test('footnotes share the core case and whitespace identity rule for insertion, numbering and rename', () => {
  const notices = [];
  const editor = createEditor(
    [
      {
        type: 'paragraph',
        attrs: { id: 'p' },
        content: [{ type: 'footnoteReference', attrs: { label: 'FIRST   Note' } }],
      },
      {
        type: 'footnoteDefinition',
        attrs: { id: 'd1', label: 'First Note' },
        content: [paragraph('body', '内容')],
      },
    ],
    notices,
  );
  try {
    assert.equal(footnoteIndex(editor.state.doc).order.get('first note'), 1);
    assert.equal(footnoteIndex(editor.state.doc).definitions.get('first note'), 1);
    assert.throws(() => insertFootnote(editor.state, ' first  note ', '新内容'), /已有定义/);
    const definition = nodes(editor, 'footnoteDefinition')[0];
    editor.view.dispatch(renameFootnoteDefinition(editor.state, definition.position, 'Renamed'));
    assert.equal(nodes(editor, 'footnoteReference')[0].node.attrs.label, 'Renamed');
    assert.equal(nodes(editor, 'footnoteDefinition')[0].node.attrs.id, 'd1');
    const before = editor.getJSON();
    editor.commands.insertContentAt(editor.state.doc.content.size, {
      type: 'footnoteDefinition',
      attrs: { id: 'duplicate', label: ' renamed ' },
      content: [paragraph('new', '重复')],
    });
    assert.deepEqual(editor.getJSON(), before);
    assert.match(notices.at(-1), /脚注标识/);
  } finally {
    editor.destroy();
  }
});

test('invalid callout control characters and embed dimensions are rejected before changing the document', () => {
  const notices = [];
  const editor = createEditor([paragraph('p', '保留')], notices);
  try {
    const before = editor.getJSON();
    editor.commands.insertContentAt(editor.state.doc.content.size, {
      type: 'callout',
      attrs: { id: 'c', callout_type: 'note', title: '标题\u0085' },
      content: [paragraph('body', '正文')],
    });
    assert.deepEqual(editor.getJSON(), before);
    assert.match(notices.at(-1), /控制字符/);
    editor.commands.insertContentAt(editor.state.doc.content.size, {
      type: 'wikiEmbed',
      attrs: {
        id: 'embed',
        wiki_target: '目标',
        occurrence_id: crypto.randomUUID(),
        embed_width: 4097,
      },
    });
    assert.deepEqual(editor.getJSON(), before);
    assert.match(notices.at(-1), /4096/);
  } finally {
    editor.destroy();
  }
});

test('replacing the whole document can replace its leading YAML without flattening it to text', () => {
  const editor = createEditor([
    { type: 'frontmatter', attrs: { id: 'yaml', source: 'old: true' } },
    paragraph('p', '旧正文'),
  ]);
  const notices = [];
  try {
    editor.commands.selectAll();
    insertEditorMarkdown(editor, '---\ntitle: 新属性\n---\n\n新正文', (notice) =>
      notices.push(notice),
    );
    assert.equal(editor.state.doc.firstChild.type.name, 'frontmatter');
    assert.equal(editor.state.doc.firstChild.attrs.source, 'title: 新属性');
    assert.equal(editor.state.doc.lastChild.textContent, '新正文');
    assert.equal(notices.at(-1), null);
  } finally {
    editor.destroy();
  }
});

test('large Markdown paste is handed off intact; plain paste bypasses conversion', () => {
  const editor = createEditor([paragraph('original', '原文')]);
  const prepared = [];
  try {
    const extension = richMarkdownPaste(
      () => {},
      (source) => prepared.push(source),
    );
    const [plugin] = getExtensionField(extension, 'addProseMirrorPlugins', { editor })();
    const text = '# 标题\n\n' + '保留输入'.repeat(5000);
    const event = {
      clipboardData: { files: [], getData: (type) => (type === 'text/plain' ? text : '') },
    };
    const before = editor.getJSON();
    assert.equal(plugin.props.handlePaste(editor.view, event), true);
    assert.deepEqual(prepared, [text]);
    assert.deepEqual(editor.getJSON(), before);
    plugin.props.handleKeyDown(editor.view, { ctrlKey: true, shiftKey: true, key: 'V' });
    assert.equal(plugin.props.handlePaste(editor.view, event), false);
    assert.equal(prepared.length, 1);
    assert.deepEqual(editor.getJSON(), before);
    const unclosed = '['.repeat(120_000);
    const unclosedEvent = {
      clipboardData: { files: [], getData: (type) => (type === 'text/plain' ? unclosed : '') },
    };
    assert.equal(plugin.props.handlePaste(editor.view, unclosedEvent), true);
    assert.equal(prepared.at(-1), unclosed);
    assert.deepEqual(editor.getJSON(), before);
  } finally {
    editor.destroy();
  }
});

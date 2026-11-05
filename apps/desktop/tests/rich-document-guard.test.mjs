import assert from 'node:assert/strict';
import test from 'node:test';
import { Editor } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import UniqueID from '@tiptap/extension-unique-id';
import { Table, TableRow } from '@tiptap/extension-table';
import TaskList from '@tiptap/extension-task-list';
import { ReplaceStep } from '@tiptap/pm/transform';
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
  richDocumentCheckForReplacement,
  richDocumentError,
  richDocumentFacts,
} from '../src/editor/rich-format-model.ts';

/**
 * `richDocumentGuard` settles a plain replacement from the range it replaced
 * instead of re-reading the whole document. That is only sound when the whole
 * document's verdict and the range's verdict are the same thing, which is what
 * this file pins down: every case below is checked both ways and must agree on
 * the error *and* on the multisets the next change will build on.
 */

const paragraph = (id, text = '', attrs = {}) => ({
  type: 'paragraph',
  attrs: { id, ...attrs },
  ...(text ? { content: [{ type: 'text', text }] } : {}),
});

function createEditor(content, notices = []) {
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
  editor.view.updateState(editor.state.reconfigure({ plugins: editor.extensionManager.plugins }));
  return editor;
}

/** Blocks in document order, with the range each one occupies. */
function blockRanges(editor) {
  const ranges = [];
  let position = 0;
  editor.state.doc.forEach((node) => {
    ranges.push({ from: position, to: position + node.nodeSize, node });
    position += node.nodeSize;
  });
  return ranges;
}

/** A stable shape for a facts record, so two of them can be compared. */
function factsKey(facts) {
  return JSON.stringify([
    [...facts.anchors.entries()].sort(),
    [...facts.footnoteLabels.entries()].sort(),
    facts.frontmatter,
  ]);
}

function factsWithFrontmatter(id = 'yaml', source = 'title: 会议') {
  return { type: 'frontmatter', attrs: { id, source } };
}

const docWith = (...blocks) => blocks;

/** Every case is one plain replacement built from the live document. */
const changes = {
  'typing inside one block': (editor) => editor.state.tr.insertText('字', 3),
  'deleting part of a block': (editor) => {
    const [first] = blockRanges(editor);
    return editor.state.tr.delete(first.from + 1, first.from + 3);
  },
  'deleting a whole block': (editor) => {
    const ranges = blockRanges(editor);
    return editor.state.tr.delete(ranges[1].from, ranges[1].to);
  },
  'replacing a block with an identical anchor': (editor) => {
    const ranges = blockRanges(editor);
    return editor.state.tr.replaceWith(
      ranges[1].from,
      ranges[1].to,
      editor.schema.nodeFromJSON(paragraph('b1', '改写', { block_id: 'anchor-2' })),
    );
  },
  'replacing a block with a paragraph that repeats an anchor': (editor) => {
    const ranges = blockRanges(editor);
    return editor.state.tr.replaceWith(
      ranges[1].from,
      ranges[1].to,
      editor.schema.nodeFromJSON(paragraph('b1', '重复', { block_id: 'anchor-1' })),
    );
  },
  'replacing a block with content that drops its anchor': (editor) => {
    const ranges = blockRanges(editor);
    return editor.state.tr.replaceWith(
      ranges[1].from,
      ranges[1].to,
      editor.schema.nodeFromJSON(paragraph('b1-fresh', '新块')),
    );
  },
  'inserting a nested YAML block': (editor) => {
    const ranges = blockRanges(editor);
    return editor.state.tr.insert(
      ranges[0].from + 2,
      editor.schema.nodeFromJSON(factsWithFrontmatter('yaml-nested')),
    );
  },
  'inserting a second YAML block at the top': (editor) => {
    const yaml = blockRanges(editor)[0];
    return editor.state.tr.insert(
      yaml.to,
      editor.schema.nodeFromJSON(factsWithFrontmatter('yaml-2')),
    );
  },
  'inserting a footnote definition that repeats a label': (editor) => {
    const ranges = blockRanges(editor);
    return editor.state.tr.insert(
      ranges[0].from,
      editor.schema.nodeFromJSON({
        type: 'footnoteDefinition',
        attrs: { label: '1' },
        content: [paragraph('note-dup', '又一处定义')],
      }),
    );
  },
  'inserting a footnote definition with a new label': (editor) => {
    const ranges = blockRanges(editor);
    return editor.state.tr.insert(
      ranges[0].from,
      editor.schema.nodeFromJSON({
        type: 'footnoteDefinition',
        attrs: { label: '9' },
        content: [paragraph('note-nine', '新定义')],
      }),
    );
  },
};

const content = () =>
  docWith(
    paragraph('a0', '第一段', { block_id: 'anchor-1' }),
    paragraph('b1', '第二段', { block_id: 'anchor-2' }),
    paragraph('c2', '第三段'),
    {
      type: 'footnoteDefinition',
      attrs: { label: '1' },
      content: [paragraph('n1', '定义')],
    },
  );

const yamlContent = () =>
  docWith(factsWithFrontmatter('yaml-1'), paragraph('a0', '第一段'), paragraph('b1', '第二段'));

for (const [name, build] of Object.entries(changes)) {
  test(`a replacement agrees with the full pass: ${name}`, () => {
    const editor = createEditor(name.includes('YAML') ? yamlContent() : content());
    try {
      const before = editor.state.doc;
      const transaction = build(editor);
      assert.equal(transaction.steps.length, 1, 'the case must stay a single step');
      const step = transaction.steps[0];
      assert.ok(step instanceof ReplaceStep, 'the case must stay a plain replacement');
      const previous = richDocumentFacts(before);
      assert.equal(previous.error, null, 'the document before the change must be valid');
      const incremental = richDocumentCheckForReplacement(
        before,
        transaction.doc,
        previous.facts,
        [step.from, step.to],
        [step.from, step.from + step.slice.size],
      );
      const full = richDocumentFacts(transaction.doc);
      assert.ok(incremental, 'a plain replacement must be modelled, not fall back');
      assert.equal(incremental.error, full.error);
      // A rejected change keeps the account of the document that is still open,
      // so the multisets are only comparable when the change is accepted.
      if (full.error === null) assert.equal(factsKey(incremental.facts), factsKey(full.facts));
    } finally {
      editor.destroy();
    }
  });
}

test('an anchor that only moved is not reported as a repeat', () => {
  const editor = createEditor(content());
  try {
    const range = blockRanges(editor)[1];
    const moved = editor.schema.nodeFromJSON(
      paragraph('b1', '第二段的替身', { block_id: 'anchor-2' }),
    );
    const transaction = editor.state.tr.replaceWith(range.from, range.to, moved);
    const incremental = richDocumentCheckForReplacement(
      editor.state.doc,
      transaction.doc,
      richDocumentFacts(editor.state.doc).facts,
      [range.from, range.to],
      [range.from, range.from + transaction.steps[0].slice.size],
    );
    assert.equal(incremental?.error, null);
    assert.equal(richDocumentFacts(transaction.doc).error, null);
    assert.equal(factsKey(incremental.facts), factsKey(richDocumentFacts(transaction.doc).facts));
  } finally {
    editor.destroy();
  }
});

test('a change that is not a plain replacement asks for the full pass', () => {
  const editor = createEditor(content());
  try {
    assert.equal(
      richDocumentCheckForReplacement(
        editor.state.doc,
        editor.state.doc,
        richDocumentFacts(editor.state.doc).facts,
        [5, 4],
        [0, 0],
      ),
      null,
    );
  } finally {
    editor.destroy();
  }
});

test('the guard still rejects a duplicate anchor and keeps the old document', () => {
  const notices = [];
  const editor = createEditor(content(), notices);
  try {
    // A command reports that it ran; the guard decides whether the document
    // moves, so the proof is the notice plus a document that stayed put.
    editor.commands.insertContentAt(editor.state.doc.content.size, {
      type: 'paragraph',
      attrs: { id: 'anchor-copy', block_id: 'anchor-1' },
      content: [{ type: 'text', text: '重复锚点' }],
    });
    assert.match(notices.at(-1), /重复块标识/);
    assert.equal(blockRanges(editor).length, 4, 'the rejected change left no trace');
    assert.equal(richDocumentError(editor.state.doc), null);
  } finally {
    editor.destroy();
  }
});

test('the guard still rejects a YAML block that is not the only top-level one', () => {
  const notices = [];
  const editor = createEditor(yamlContent(), notices);
  try {
    editor.commands.insertContentAt(
      editor.state.doc.content.size,
      factsWithFrontmatter('yaml-2', 'title: 追加'),
    );
    assert.match(notices.at(-1), /YAML 属性只能出现在文档顶部一次/);
    assert.equal(blockRanges(editor).length, 3, 'the rejected change left no trace');
    assert.equal(richDocumentError(editor.state.doc), null);
  } finally {
    editor.destroy();
  }
});

test('an attribute change is re-read in full and still rejected', () => {
  const notices = [];
  const editor = createEditor(content(), notices);
  try {
    const target = blockRanges(editor)[1];
    editor.commands.command(({ tr, dispatch }) => {
      tr.setNodeMarkup(target.from, undefined, {
        ...editor.state.doc.child(1).attrs,
        block_id: 'anchor-1',
      });
      dispatch?.(tr);
      return true;
    });
    assert.match(notices.at(-1), /重复块标识/);
    assert.equal(editor.state.doc.child(1).attrs.block_id, 'anchor-2', 'the old document stayed');
    assert.equal(richDocumentError(editor.state.doc), null);
  } finally {
    editor.destroy();
  }
});

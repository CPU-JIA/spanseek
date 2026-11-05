import assert from 'node:assert/strict';
import test from 'node:test';
import { Editor } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import { history, undo, redo } from '@tiptap/pm/history';
import { NodeSelection } from '@tiptap/pm/state';
import {
  BlockContinuation,
  continueOutsideContainer,
  exitEmptyContainer,
  moveAcrossContainer,
} from '../src/editor/block-continuation.ts';
import { Callout, RichBlockAttributes } from '../src/editor/rich-schema.ts';
import {
  OrdinaryLinkPresentation,
  captureLinkEditTarget,
  editOrdinaryLink,
  mapLinkEditTarget,
  selectedOrdinaryLink,
} from '../src/editor/link-edit-target.ts';
import { ordinaryLinkTarget, storedOrdinaryLink } from '../src/rich/link-target.ts';

const p = (text = '', id = crypto.randomUUID()) => ({
  type: 'paragraph',
  attrs: { id },
  content: text ? [{ type: 'text', text }] : [],
});
const wrapper = (type, content, attrs = {}) => ({
  type,
  attrs: {
    id: crypto.randomUUID(),
    ...(type === 'callout'
      ? { callout_type: 'custom-kind', title: '标题保留', collapsed: true }
      : {}),
    ...attrs,
  },
  content,
});
function editorFor(content) {
  const editor = new Editor({
    element: null,
    enablePasteRules: false,
    extensions: [
      StarterKit.configure({ trailingNode: false, undoRedo: false }),
      Callout,
      RichBlockAttributes,
      BlockContinuation,
      OrdinaryLinkPresentation,
    ],
    content: { type: 'doc', content },
  });
  editor.view.updateState(
    editor.state.reconfigure({ plugins: [...editor.extensionManager.plugins, history()] }),
  );
  return editor;
}
function find(editor, text) {
  let found;
  editor.state.doc.descendants((node, pos) => {
    if (node.isTextblock && node.textContent === text) found = { node, pos };
  });
  assert.ok(found, `missing textblock ${text}`);
  return found;
}
const dispatch = (editor) => (transaction) => editor.view.dispatch(transaction);

for (const type of ['blockquote', 'callout']) {
  test(`${type}: empty final Enter exits in the middle, preserving prior body and following block; undo/redo are atomic`, () => {
    const editor = editorFor([p('before'), wrapper(type, [p('protected'), p()]), p('after')]);
    try {
      const empty = find(editor, '');
      editor.commands.setTextSelection(empty.pos + 1);
      const original = editor.getJSON();
      assert.equal(exitEmptyContainer(editor.state, dispatch(editor)), true);
      assert.deepEqual(
        editor.getJSON().content.map((node) => node.type),
        ['paragraph', type, 'paragraph', 'paragraph'],
      );
      assert.equal(editor.state.doc.child(1).textContent, 'protected');
      assert.equal(editor.state.doc.lastChild.textContent, 'after');
      assert.equal(editor.state.selection.$from.depth, 1);
      assert.equal(undo(editor.state, dispatch(editor)), true);
      assert.deepEqual(editor.getJSON(), original);
      assert.equal(redo(editor.state, dispatch(editor)), true);
      assert.equal(editor.state.doc.child(1).textContent, 'protected');
    } finally {
      editor.destroy();
    }
  });
  test(`${type}: arrows at the document edges create an outside paragraph, and normal internal arrows do nothing`, () => {
    const editor = editorFor([wrapper(type, [p('line one'), p('line two')])]);
    try {
      const first = find(editor, 'line one');
      editor.commands.setTextSelection(first.pos + 3);
      assert.equal(moveAcrossContainer(editor.state, 1, dispatch(editor)), false);
      const last = find(editor, 'line two');
      editor.commands.setTextSelection(last.pos + 1 + last.node.content.size);
      assert.equal(moveAcrossContainer(editor.state, 1, dispatch(editor)), true);
      assert.equal(editor.state.doc.childCount, 2);
      assert.equal(editor.state.doc.firstChild.textContent, 'line oneline two');
      assert.equal(editor.state.selection.$from.depth, 1);
      editor.commands.setTextSelection(find(editor, 'line one').pos + 1);
      assert.equal(moveAcrossContainer(editor.state, -1, dispatch(editor)), true);
      assert.equal(editor.state.doc.firstChild.type.name, 'paragraph');
      assert.equal(editor.state.doc.child(1).type.name, type);
    } finally {
      editor.destroy();
    }
  });
}

test('nested callout/quote exits only the inner level; empty list paragraphs remain owned by list keymaps', () => {
  const editor = editorFor([
    wrapper('callout', [p('outer body'), wrapper('blockquote', [p('inner body'), p()])]),
  ]);
  try {
    editor.commands.setTextSelection(find(editor, '').pos + 1);
    assert.equal(exitEmptyContainer(editor.state, dispatch(editor)), true);
    assert.equal(editor.state.doc.childCount, 1);
    assert.equal(editor.state.selection.$from.depth, 2);
    assert.equal(editor.state.doc.firstChild.lastChild.type.name, 'paragraph');
    assert.equal(exitEmptyContainer(editor.state, dispatch(editor)), true);
    assert.equal(editor.state.doc.childCount, 2);
    assert.equal(editor.state.doc.firstChild.textContent, 'outer bodyinner body');
    assert.equal(editor.state.doc.firstChild.attrs.title, '标题保留');
  } finally {
    editor.destroy();
  }
  const list = editorFor([
    wrapper('callout', [{ type: 'bulletList', content: [{ type: 'listItem', content: [p()] }] }]),
  ]);
  try {
    list.commands.setTextSelection(find(list, '').pos + 1);
    assert.equal(exitEmptyContainer(list.state, dispatch(list)), false);
    assert.equal(list.commands.liftListItem('listItem'), true);
    assert.equal(exitEmptyContainer(list.state, dispatch(list)), true);
    assert.equal(list.state.selection.$from.depth, 1);
  } finally {
    list.destroy();
  }
});

test('empty custom callout and a selected folded block retain their attributes and body on Continue', () => {
  const editor = editorFor([wrapper('callout', [p()])]);
  try {
    const original = editor.state.doc.firstChild;
    editor.commands.setTextSelection(2);
    assert.equal(exitEmptyContainer(editor.state, dispatch(editor)), true);
    assert.ok(editor.state.doc.firstChild.eq(original));
    editor.view.dispatch(editor.state.tr.setSelection(NodeSelection.create(editor.state.doc, 0)));
    assert.equal(continueOutsideContainer(editor.state, dispatch(editor)), true);
    assert.equal(editor.state.doc.childCount, 2, 'reuse an adjacent empty paragraph');
    editor.view.dispatch(editor.state.tr.setSelection(NodeSelection.create(editor.state.doc, 0)));
    editor.commands.insertContent({
      type: 'paragraph',
      content: [{ type: 'text', text: 'replacement' }],
    });
    assert.equal(editor.state.doc.firstChild.textContent, 'replacement');
    assert.equal(editor.state.doc.childCount, 2);
  } finally {
    editor.destroy();
  }
});

test('ordinary links classify supported targets and reject script/file/empty/malformed schemes', () => {
  for (const href of [
    'https://example.com/path?q=1#part',
    'HTTP://example.com',
    'mailto:person@example.com?subject=Hello',
  ])
    assert.equal(ordinaryLinkTarget(href).kind, 'external');
  assert.deepEqual(ordinaryLinkTarget('#%E6%A0%87%E9%A2%98'), {
    kind: 'anchor',
    href: '#%E6%A0%87%E9%A2%98',
    anchor: '标题',
  });
  for (const value of [
    '',
    'javascript:alert(1)',
    'data:text/html,a',
    'file:///C:/private',
    'https:',
    'https:/example.com',
    'http://',
    'mailto:',
    'https://exa mple.com',
    '#%',
    'https://example.com/\u0000',
  ])
    assert.throws(() => ordinaryLinkTarget(value), undefined, value);
  assert.throws(() => storedOrdinaryLink('#heading'), /内部链接/);
});

const linked = (text, href, extra = {}) => ({
  type: 'text',
  text,
  marks: [
    { type: 'bold' },
    { type: 'link', attrs: { href, target: '_blank', rel: 'noopener noreferrer', ...extra } },
  ],
});
test('a link action on code or a nontext block is rejected instead of reporting a no-op as success', () => {
  const editor = editorFor([
    {
      type: 'paragraph',
      content: [{ type: 'text', text: 'inline code', marks: [{ type: 'code' }] }],
    },
    { type: 'horizontalRule' },
  ]);
  try {
    editor.commands.setTextSelection({ from: 1, to: 7 });
    const original = editor.getJSON();
    assert.throws(
      () =>
        editOrdinaryLink(editor.state, captureLinkEditTarget(editor.state), 'https://example.com'),
      /普通正文/,
    );
    editor.commands.setTextSelection(4);
    assert.throws(
      () =>
        editOrdinaryLink(editor.state, captureLinkEditTarget(editor.state), 'https://example.com'),
      /普通正文/,
    );
    editor.commands.setNodeSelection(editor.state.doc.firstChild.nodeSize);
    assert.throws(
      () =>
        editOrdinaryLink(editor.state, captureLinkEditTarget(editor.state), 'https://example.com'),
      /普通正文/,
    );
    assert.deepEqual(editor.getJSON(), original);
  } finally {
    editor.destroy();
  }
});
test('edit/remove a link updates its complete mark range without losing text, other marks or presentation attributes', () => {
  const editor = editorFor([
    {
      type: 'paragraph',
      content: [
        linked('ordinary link', 'https://old.example', { title: '旧链接标题', target: '_self' }),
        { type: 'text', text: ' unrelated' },
      ],
    },
  ]);
  try {
    editor.commands.setTextSelection(5);
    const target = captureLinkEditTarget(editor.state);
    assert.equal(target.from, 1);
    assert.equal(target.to, 14);
    editor.view.dispatch(editOrdinaryLink(editor.state, target, 'https://new.example/path'));
    const link = selectedOrdinaryLink(editor.state);
    assert.equal(link.href, 'https://new.example/path');
    assert.equal(link.attrs.rel, 'noopener noreferrer');
    assert.equal(link.attrs.title, '旧链接标题');
    assert.equal(link.attrs.target, '_self');
    assert.equal(editor.state.doc.textContent, 'ordinary link unrelated');
    assert.equal(
      editor.state.doc.firstChild.firstChild.marks.some((mark) => mark.type.name === 'bold'),
      true,
    );
    editor.commands.setTextSelection(4);
    editor.view.dispatch(editOrdinaryLink(editor.state, captureLinkEditTarget(editor.state), null));
    assert.equal(
      editor.state.doc.firstChild.firstChild.marks.some((mark) => mark.type.name === 'link'),
      false,
    );
    assert.equal(editor.state.doc.textContent, 'ordinary link unrelated');
    assert.equal(undo(editor.state, dispatch(editor)), true);
    assert.equal(editor.state.doc.textContent, 'ordinary link unrelated');
  } finally {
    editor.destroy();
  }
});

test('link dialog target follows unrelated edits and rejects replaced selected content instead of applying to its neighbour', () => {
  const editor = editorFor([
    p('before'),
    { type: 'paragraph', content: [linked('link', 'https://example.com')] },
  ]);
  try {
    editor.commands.setTextSelection(find(editor, 'link').pos + 2);
    let target = captureLinkEditTarget(editor.state);
    let transaction = editor.state.tr.insertText('prefix ', 1);
    target = mapLinkEditTarget(target, transaction);
    editor.view.dispatch(transaction);
    assert.doesNotThrow(() => editOrdinaryLink(editor.state, target, 'https://example.net'));
    transaction = editor.state.tr.insertText('different', target.from, target.to);
    target = mapLinkEditTarget(target, transaction);
    editor.view.dispatch(transaction);
    assert.throws(() => editOrdinaryLink(editor.state, target, 'https://example.net'), /已改变/);
    assert.equal(editor.state.doc.lastChild.textContent, 'different');
  } finally {
    editor.destroy();
  }
});

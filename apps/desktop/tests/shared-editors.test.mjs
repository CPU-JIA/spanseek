import assert from 'node:assert/strict';
import test from 'node:test';
import { Schema } from '@tiptap/pm/model';
import { EditorState } from '@tiptap/pm/state';
import { joinEditors, shareEditedDocument } from '../src/editor/shared-editors.ts';
import { needsDocumentSync } from '../src/editor/document-sync.ts';

function schema() {
  return new Schema({
    nodes: {
      doc: { content: 'paragraph+' },
      paragraph: { content: 'text*' },
      text: {},
    },
  });
}

// Only the editor surface joinEditors touches is replaced; the documents are real
// ProseMirror states so mirrored steps are applied exactly as in the product.
function pane(paneSchema, text) {
  let state = EditorState.create({
    schema: paneSchema,
    doc: paneSchema.node('doc', null, [
      paneSchema.node('paragraph', null, [paneSchema.text(text)]),
    ]),
  });
  const listeners = new Set();
  const editor = {
    isEditable: true,
    isDestroyed: false,
    get state() {
      return state;
    },
    get text() {
      return state.doc.textContent;
    },
    setEditable(value) {
      editor.isEditable = value;
    },
    view: {
      dispatch(transaction) {
        state = state.apply(transaction);
      },
    },
    on(_name, listener) {
      listeners.add(listener);
    },
    off(_name, listener) {
      listeners.delete(listener);
    },
    emit(transaction) {
      for (const listener of [...listeners]) listener({ transaction, appendedTransactions: [] });
    },
    type(character) {
      const transaction = state.tr.insertText(character, 2);
      state = state.apply(transaction);
      editor.emit(transaction);
    },
  };
  return editor;
}

test('a mirrored pane is marked with the document object it now holds', () => {
  const pmSchema = schema();
  const typed = pane(pmSchema, '原文');
  const mirror = pane(pmSchema, '原文');
  const controller = {};
  const marked = [];
  const stopTyped = joinEditors(controller, typed, {
    fail: (error) => assert.fail(`typed pane mirror failed: ${error.message}`),
    applied: () => assert.fail('the pane that typed must not mark itself'),
  });
  const stopMirror = joinEditors(controller, mirror, {
    fail: (error) => assert.fail(`mirrored pane failed: ${error.message}`),
    applied: (document) => marked.push(document),
  });

  typed.type('新');
  assert.equal(typed.text, '原新文');
  assert.equal(mirror.text, '原新文');

  // The pane that typed publishes the object its onUpdate produced.
  const produced = typed.state.doc.toJSON();
  assert.deepEqual(produced, mirror.state.doc.toJSON());
  shareEditedDocument(controller, typed, produced);

  assert.equal(marked.length, 1);
  assert.equal(marked[0], produced, 'the mirrored pane holds exactly the produced object');
  assert.deepEqual(marked[0], mirror.state.doc.toJSON(), 'that object is the mirrored document');
  assert.equal(
    needsDocumentSync(produced, marked[0], () => assert.fail('identity must settle the mirror')),
    false,
  );
  stopTyped();
  stopMirror();
});

test('a pane whose mirror failed is never marked as holding the newer document', () => {
  const pmSchema = schema();
  const typed = pane(pmSchema, '原文');
  const stale = pane(pmSchema, '本地尚未同步');
  const controller = {};
  const failures = [];
  const marked = [];
  const stopTyped = joinEditors(controller, typed, {
    fail: () => assert.fail('the typed pane must not report its own mirror'),
    applied: () => assert.fail('the pane that typed must not mark itself'),
  });
  const stopStale = joinEditors(controller, stale, {
    fail: (error) => failures.push(error),
    applied: (document) => marked.push(document),
  });

  typed.type('新');
  assert.equal(failures.length, 1, 'a diverged pane fails explicitly');
  assert.equal(stale.isEditable, false);
  assert.equal(stale.text, '本地尚未同步', 'the diverged pane keeps its own text');

  shareEditedDocument(controller, typed, typed.state.doc.toJSON());
  assert.deepEqual(
    marked,
    [],
    'marking a pane that never received the edit would hide a needed document sync',
  );
  stopTyped();
  stopStale();
});

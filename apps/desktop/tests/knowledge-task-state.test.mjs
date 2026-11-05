import test from 'node:test';
import assert from 'node:assert/strict';
import {
  emptyTaskDraft,
  parseTaskDraft,
  mergeTaskMaterials,
} from '../src/inference/knowledge-task-state.ts';

test('a custom task draft retains editable instructions, versioned template and material snapshot', () => {
  const draft = {
    ...emptyTaskDraft(),
    title: '比较两个方案',
    instructions: '按预算和适用条件比较。',
    template: { id: 'custom-id', row_version: 3 },
    templateName: '方案比较',
    materials: [
      { id: 'evidence-A', title: '方案 A', text: '原始片段', target: { object_id: 'note-A' } },
    ],
    retryOf: 'previous-task',
  };
  const serialized = JSON.stringify(draft);
  draft.instructions = 'later edit';
  draft.materials[0].text = 'later source change';
  const restored = parseTaskDraft(serialized);
  assert.equal(restored.instructions, '按预算和适用条件比较。');
  assert.equal(restored.materials[0].text, '原始片段');
  assert.equal(restored.template.row_version, 3);
  assert.equal(restored.retryOf, 'previous-task');
});

test('a task can be restored without any template', () => {
  assert.deepEqual(parseTaskDraft(JSON.stringify(emptyTaskDraft())), emptyTaskDraft());
});

test('context additions update the selected source in place without dropping other work', () => {
  const one = { id: 'old-A', target: { object_id: 'A' } };
  const two = { id: 'keep-B', target: { object_id: 'B' } };
  const quote = {
    id: 'quote-A',
    target: { object_id: 'A' },
    annotation: { annotation_id: 'quote' },
  };
  const fresh = { id: 'new-A', target: { object_id: 'A' } };
  const draft = {
    ...emptyTaskDraft(),
    instructions: 'Keep my instructions',
    materials: [one, two, quote],
  };
  const updated = mergeTaskMaterials(
    draft,
    [fresh],
    ['A'],
    [
      {
        object_id: 'A',
        title: 'A',
        status: 'limited',
        total_characters: 100,
        included_characters: 20,
        included_fragments: 1,
      },
    ],
  );
  assert.equal(updated.instructions, draft.instructions);
  assert.deepEqual(
    updated.materials.map((e) => e.id),
    ['new-A', 'keep-B', 'quote-A'],
  );
  assert.equal(updated.materialWarningsAccepted, false);
  assert.deepEqual(
    draft.materials.map((e) => e.id),
    ['old-A', 'keep-B', 'quote-A'],
  );
  assert.deepEqual(
    parseTaskDraft(JSON.stringify({ ...updated, materials: [] })).materialWarnings,
    updated.materialWarnings,
  );
});

test('unreadable or incomplete drafts fail explicitly instead of being silently accepted', () => {
  for (const value of [
    null,
    [],
    { title: 'missing fields' },
    { ...emptyTaskDraft(), template: { id: 'bad', row_version: 0 } },
    { ...emptyTaskDraft(), materials: [{ id: 'missing-source' }] },
    { ...emptyTaskDraft(), retryOf: {} },
  ]) {
    assert.throws(() => parseTaskDraft(JSON.stringify(value)), /Invalid saved/);
  }
  assert.throws(() => parseTaskDraft('{not json'));
});

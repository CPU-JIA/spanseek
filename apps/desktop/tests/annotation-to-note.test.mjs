import test from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { DraftController } from '../src/editor/draft-controller.ts';
import {
  appendAnnotationToNote,
  containsAnnotationIntent,
  containsAnyAnnotationIntent,
  createAnnotationNoteAction,
  createAnnotationNoteIntent,
  pendingAnnotationNoteAction,
  validateAnnotationNoteAction,
  validateAnnotationNoteIntent,
} from '../src/sources/annotation-to-note.ts';

async function fixture(failFirst = false, firstError = new Error('receipt lost')) {
  const identity = { local_instance_id: randomUUID(), workspace_id: randomUUID() };
  const note = {
    id: randomUUID(),
    title: 'Working note',
    revision: 1,
    revision_id: randomUUID(),
    parent_revision_id: null,
    content_hash: '',
    author_kind: 'user',
    schema_version: 2,
    trashed: false,
    updated_at: '2026-10-03T00:00:00Z',
    document: {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          attrs: { id: randomUUID() },
          content: [{ type: 'text', text: 'Existing content' }],
        },
      ],
    },
  };
  const records = new Map();
  const storage = {
    async read(key) {
      return structuredClone(records.get(key));
    },
    async write(record) {
      records.set(record.key, structuredClone(record));
    },
  };
  const attempts = [];
  const save = async (commandId, request) => {
    attempts.push({ commandId, request: structuredClone(request) });
    if (failFirst && attempts.length === 1) throw firstError;
    const revision = randomUUID();
    return {
      note: {
        ...note,
        title: request.title,
        document: request.document,
        revision_id: revision,
        revision: 2,
      },
      receipt: {
        command_id: commandId,
        object_id: note.id,
        revision_id: revision,
        change_seq: '2',
        committed_at: note.updated_at,
        replayed: attempts.length > 1,
      },
    };
  };
  const controller = await DraftController.open(identity, note, storage, save);
  const annotation = {
    id: randomUUID(),
    source_version_id: randomUUID(),
    row_version: 1,
    trashed: false,
    comment: 'My explanation',
    locator_schema: 1,
    locator: {
      kind: 'text',
      source_version_id: '',
      block_id: randomUUID(),
      start_codepoint: 0,
      end_codepoint: 7,
      quote: '原文😀\n第二行',
      prefix: '',
      suffix: '',
    },
  };
  annotation.locator.source_version_id = annotation.source_version_id;
  const intent = createAnnotationNoteIntent(
    identity,
    annotation,
    note.id,
    note.title,
    'Original excerpt',
  );
  return { identity, controller, intent, attempts, annotation, storage, save, note };
}

function secondAnnotation(first) {
  const sourceVersionId = randomUUID();
  return {
    ...first,
    id: randomUUID(),
    source_version_id: sourceVersionId,
    row_version: 4,
    comment: 'Comment on page iv',
    locator: {
      kind: 'pdf',
      source_version_id: sourceVersionId,
      page_index: 3,
      page_label: 'iv',
      crop_box: [0, 0, 612, 792],
      rotation: 90,
      coordinate_space: 'pdf_user_space',
      rects: [[10, 20, 40, 60]],
      quote: 'Rotated page quote',
      prefix: '',
      suffix: '',
    },
  };
}

test('unknown receipt retries the same save without duplicate quote or source occurrence', async () => {
  const { controller, intent, attempts } = await fixture(true);
  let checks = 0;
  await assert.rejects(
    appendAnnotationToNote(controller, intent, async () => {
      checks++;
    }),
    /receipt lost/,
  );
  await appendAnnotationToNote(controller, intent, async () => {
    checks++;
  });
  assert.equal(attempts.length, 2);
  assert.equal(attempts[0].commandId, attempts[1].commandId);
  assert.deepEqual(attempts[0].request, attempts[1].request);
  assert.equal(checks, 1);
  const document = controller.getSnapshot().record.document;
  assert.equal(document.content.filter((node) => node.type === 'blockquote').length, 1);
  assert.equal(document.content.at(-1).content[0].text, 'My explanation');
  assert(containsAnnotationIntent(controller.getSnapshot().record.base.document, intent));
});

test('source verification preserves concurrent shared edits and validates workspace', async () => {
  const { controller, intent, identity } = await fixture();
  await appendAnnotationToNote(controller, intent, async () => {
    const record = controller.getSnapshot().record;
    controller.update('Renamed in other pane', {
      ...record.document,
      content: [
        ...record.document.content,
        {
          type: 'paragraph',
          attrs: { id: randomUUID() },
          content: [{ type: 'text', text: 'Concurrent edit' }],
        },
      ],
    });
  });
  assert.equal(controller.getSnapshot().record.title, 'Renamed in other pane');
  assert.equal(
    controller.getSnapshot().record.document.content[1].content[0].text,
    'Concurrent edit',
  );
  assert.throws(
    () =>
      validateAnnotationNoteIntent(intent, intent.key, {
        ...identity,
        local_instance_id: randomUUID(),
      }),
    /does not match/,
  );
});

test('unavailable source never appends, and user-removed pending occurrences are not reinserted', async () => {
  const { controller, intent } = await fixture(true);
  await assert.rejects(
    appendAnnotationToNote(controller, intent, async () => {
      throw new Error('source unavailable');
    }),
    /source unavailable/,
  );
  assert.equal(controller.getSnapshot().record.document.content.length, 1);
  await assert.rejects(
    appendAnnotationToNote(controller, intent, async () => {}),
    /receipt lost/,
  );
  const record = controller.getSnapshot().record;
  controller.update(record.title, {
    ...record.document,
    content: record.document.content.slice(0, 1),
  });
  await assert.rejects(
    appendAnnotationToNote(controller, intent, async () => {}),
    /removed from the draft/,
  );
});

test('a batch saves once with independent fixed versions, PDF page label, quotes, and comments', async () => {
  const { controller, identity, note, annotation, attempts } = await fixture();
  const second = secondAnnotation(annotation);
  const action = createAnnotationNoteAction(
    identity,
    [annotation, second],
    note.id,
    note.title,
    'Original',
  );
  assert.equal(action.schema, 2);
  second.locator.quote = 'Changed after choosing';
  validateAnnotationNoteAction(action, identity);
  await appendAnnotationToNote(controller, action, async () => {});
  assert.equal(attempts.length, 1);
  const saved = attempts[0].request.document;
  assert.equal(saved.content.filter((node) => node.type === 'blockquote').length, 2);
  for (const item of action.items) {
    assert(containsAnnotationIntent(saved, item));
    const reference = item.nodes[1].content[0];
    assert.equal(reference.attrs.target_revision_id, item.sourceVersionId);
    assert.equal(item.locator.source_version_id, item.sourceVersionId);
  }
  assert.equal(action.items[1].locator.quote, 'Rotated page quote');
  assert.equal(action.items[1].nodes[1].content[0].attrs.label, 'Original · p. iv');
  assert.equal(saved.content.at(-1).content[0].text, 'Comment on page iv');
});

test('a batch unknown receipt survives controller reopen and retries the identical command once', async () => {
  const { controller, identity, note, annotation, attempts, storage, save } = await fixture(true);
  const action = createAnnotationNoteAction(
    identity,
    [annotation, secondAnnotation(annotation)],
    note.id,
    note.title,
    'Original',
  );
  await assert.rejects(
    appendAnnotationToNote(controller, action, async () => {}),
    /receipt lost/,
  );
  const reopened = await DraftController.open(identity, note, storage, save);
  await appendAnnotationToNote(reopened, structuredClone(action), async () => {
    assert.fail(
      'already appended batch must retry its original payload without rechecking mutable sources',
    );
  });
  assert.equal(attempts.length, 2);
  assert.equal(attempts[0].commandId, attempts[1].commandId);
  assert.deepEqual(attempts[0].request, attempts[1].request);
  assert.equal(
    reopened.getSnapshot().record.base.document.content.filter((node) => node.type === 'blockquote')
      .length,
    2,
  );
});

test('one unavailable source prevents the whole batch and partial user removals are never reinserted', async () => {
  const { controller, identity, note, annotation, attempts } = await fixture(true);
  const action = createAnnotationNoteAction(
    identity,
    [annotation, secondAnnotation(annotation)],
    note.id,
    note.title,
    'Original',
  );
  await assert.rejects(
    appendAnnotationToNote(controller, action, async () => {
      throw new Error('second source unavailable');
    }),
    /second source unavailable/,
  );
  assert.equal(attempts.length, 0);
  assert.equal(controller.getSnapshot().record.document.content.length, 1);
  await assert.rejects(
    appendAnnotationToNote(controller, action, async () => {}),
    /receipt lost/,
  );
  const { record } = controller.getSnapshot();
  controller.update(record.title, {
    type: 'doc',
    content: [record.document.content[0], ...action.items[0].nodes],
  });
  await assert.rejects(
    appendAnnotationToNote(controller, action, async () => {}),
    /removed from the draft/,
  );
  assert.equal(attempts.length, 1);
});

test('a batch is rechecked after asynchronous source validation before touching concurrent partial edits', async () => {
  const { controller, identity, note, annotation } = await fixture();
  const action = createAnnotationNoteAction(
    identity,
    [annotation, secondAnnotation(annotation)],
    note.id,
    note.title,
    'Original',
  );
  await assert.rejects(
    appendAnnotationToNote(controller, action, async () => {
      controller.update(note.title, { type: 'doc', content: action.items[0].nodes });
    }),
    /removed from the draft/,
  );
  assert.equal(containsAnyAnnotationIntent(controller.getSnapshot().record.document, action), true);
  assert.equal(
    containsAnnotationIntent(controller.getSnapshot().record.document, action.items[1]),
    false,
  );
  // Settle the deliberate concurrent edit; the rejected operation did not add the other item.
  await controller.flush();
});

test('any batch member resumes its whole pending action, preserving legacy singles and instance isolation', async () => {
  const { identity, note, annotation, intent } = await fixture();
  const second = secondAnnotation(annotation);
  const action = createAnnotationNoteAction(
    identity,
    [annotation, second],
    note.id,
    note.title,
    'Original',
  );
  assert.equal(pendingAnnotationNoteAction([action], identity, [second.id]), action);
  assert.equal(
    pendingAnnotationNoteAction([action], { ...identity, local_instance_id: randomUUID() }, [
      second.id,
    ]),
    undefined,
  );
  const legacy = structuredClone(intent);
  delete legacy.locator;
  delete legacy.nodes[1].content[0].attrs.target_revision_id;
  validateAnnotationNoteAction(legacy, identity);
  assert.equal(pendingAnnotationNoteAction([legacy], identity, [annotation.id, second.id]), legacy);
  const otherLegacy = createAnnotationNoteIntent(identity, second, note.id, note.title, 'Original');
  assert.throws(
    () => pendingAnnotationNoteAction([legacy, otherLegacy], identity, [annotation.id, second.id]),
    /separate pending actions/,
  );
  assert.equal(
    pendingAnnotationNoteAction([legacy, otherLegacy], identity, [second.id]),
    otherLegacy,
  );
});

test('selection limits, duplicate ids, corrupt batch destinations, and future intent schemas fail closed', async () => {
  const { identity, note, annotation } = await fixture();
  const make = (items) =>
    createAnnotationNoteAction(identity, items, note.id, note.title, 'Original');
  const hundred = Array.from({ length: 100 }, () => secondAnnotation(annotation));
  validateAnnotationNoteAction(make(hundred), identity);
  assert.throws(() => make([]), /1 to 100/);
  assert.throws(() => make([...hundred, annotation]), /1 to 100/);
  assert.throws(() => make([annotation, annotation]), /distinct/);
  const action = make([annotation, hundred[0]]);
  action.items[1].noteId = randomUUID();
  assert.throws(() => validateAnnotationNoteAction(action, identity), /different destination/);
  assert.throws(
    () => validateAnnotationNoteAction({ ...action, schema: 3 }, identity),
    /does not match/,
  );
  const wrongVersion = make([annotation, hundred[0]]);
  wrongVersion.items[0].nodes[1].content[0].attrs.target_revision_id = randomUUID();
  assert.throws(() => validateAnnotationNoteAction(wrongVersion, identity), /does not match/);
});

test('source checking holds the shared controller against workspace switches and releases after failure', async () => {
  const { controller, intent } = await fixture();
  await assert.rejects(
    appendAnnotationToNote(controller, intent, async () => {
      assert.throws(() => controller.freeze(), /请等待/);
      throw new Error('source check failed');
    }),
    /source check failed/,
  );
  const thaw = controller.freeze();
  thaw();
});

test('a revision conflict retains the whole batch without issuing another save or appending again', async () => {
  const conflict = Object.assign(new Error('revision conflict'), {
    details: { code: 'revision_conflict' },
  });
  const { controller, identity, note, annotation, attempts } = await fixture(true, conflict);
  const action = createAnnotationNoteAction(
    identity,
    [annotation, secondAnnotation(annotation)],
    note.id,
    note.title,
    'Original',
  );
  await assert.rejects(
    appendAnnotationToNote(controller, action, async () => {}),
    /revision conflict/,
  );
  const pending = controller.getSnapshot().record.pending;
  assert.equal(controller.getSnapshot().status, 'conflict');
  await assert.rejects(
    appendAnnotationToNote(controller, action, async () => {}),
    /resolve.*conflict/,
  );
  assert.equal(attempts.length, 1);
  assert.deepEqual(controller.getSnapshot().record.pending, pending);
  assert.equal(
    controller.getSnapshot().record.document.content.filter((node) => node.type === 'blockquote')
      .length,
    2,
  );
});

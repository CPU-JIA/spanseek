import test from 'node:test';
import assert from 'node:assert/strict';
import { convertSourceNote } from '../src/sources/source-note-conversion.ts';
import { CreateActions, createKey } from '../src/organization/create-actions.ts';

test('file conversion retains Markdown structure, unsupported source, and TXT line boundaries', () => {
  const text =
    '\uFEFF# 中文标题\r\n\r\n**粗体**\r\n\r\n![原图片](private.png)\r\n\r\n<script>visible source</script>';
  const rich = convertSourceNote(text, 'markdown');
  assert.deepEqual(
    rich.document.content.map((n) => n.type),
    ['heading', 'paragraph', 'paragraph', 'paragraph'],
  );
  assert.equal(rich.document.content[1].content[0].marks[0].type, 'bold');
  assert.equal(rich.document.content[2].content[0].text, '![原图片](private.png)');
  assert.equal(rich.document.content[3].content[0].text, '<script>visible source</script>');
  assert.ok(rich.preserved.includes('image'));
  const plain = convertSourceNote('\uFEFF# literal\r\n\r\n中文😀\r末行', 'text').document;
  assert.deepEqual(
    plain.content.map((p) => p.content.map((n) => n.text).join('')),
    ['# literal', '', '中文😀', '末行'],
  );
  assert.throws(() => convertSourceNote('not supported', 'pdf'));
});

test('source note request survives a lost receipt and remains pending until the created note opens', async () => {
  const identity = { local_instance_id: 'local', workspace_id: 'workspace' };
  let record;
  const storage = {
    read: async () => structuredClone(record),
    write: async (value) => {
      record = structuredClone(value);
    },
    remove: async () => {
      record = undefined;
    },
  };
  const command = {
    kind: 'sources_create_note',
    payload: {
      command_id: 'command-a',
      request: {
        source_version_id: 'version-old',
        source_blob_hash: 'fixed-hash',
        title: 'Imported',
        document_schema: 2,
        document: convertSourceNote('# title', 'markdown').document,
      },
    },
  };
  const expected = structuredClone(command);
  const sent = [];
  let first = true;
  const result = {
    note: { id: 'note-a', revision_id: 'revision-a' },
    receipt: { command_id: 'command-a', object_id: 'note-a', revision_id: 'revision-a' },
  };
  const send = async (value) => {
    sent.push(structuredClone(value));
    if (first) {
      first = false;
      throw new Error('lost receipt');
    }
    return result;
  };
  let controller = await CreateActions.open(identity, storage, send);
  assert.equal(await controller.create(command), null);
  command.payload.request.source_version_id = 'newer-version';
  assert.equal(record.key, createKey(identity));
  controller = await CreateActions.open(identity, storage, send);
  assert.deepEqual(await controller.create(), result);
  assert.deepEqual(sent, [expected, expected]);
  assert.ok(record.result);
  assert.equal(await controller.acknowledgeOpened('wrong-note'), false);
  assert.equal(await controller.acknowledgeOpened('note-a'), true);
  assert.equal(record, undefined);
});

test('source note definitive rejection releases its intent while IO uncertainty preserves it', async () => {
  const identity = { local_instance_id: 'local', workspace_id: 'workspace' };
  for (const code of ['invalid_document', 'object_trashed', 'not_found', 'io_failed']) {
    let record;
    const storage = {
      read: async () => structuredClone(record),
      write: async (value) => {
        record = structuredClone(value);
      },
      remove: async () => {
        record = undefined;
      },
    };
    const command = {
      kind: 'sources_create_note',
      payload: {
        command_id: `create-${code}`,
        request: {
          source_version_id: 'source-version',
          source_blob_hash: 'hash',
          title: 'title',
          document_schema: 2,
          document: convertSourceNote('body', 'text').document,
        },
      },
    };
    const controller = await CreateActions.open(identity, storage, async () => {
      const error = new Error(code);
      error.details = { code };
      throw error;
    });
    assert.equal(await controller.create(command), null);
    if (code === 'io_failed') assert.deepEqual(record.command, command);
    else assert.equal(record, undefined, code);
  }
});

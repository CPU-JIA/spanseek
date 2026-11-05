import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { SourceActions } from '../src/sources/source-actions.ts';
import {
  SOURCE_FILE_FORMATS,
  prepareScannedEntries,
  prepareSourceFiles,
  scanSkipReason,
  splitScannedEntries,
  submitSourceFiles,
} from '../src/sources/source-import-batch.ts';
import { SourceListEpoch } from '../src/sources/source-list-epoch.ts';

const identity = { local_instance_id: 'local', workspace_id: 'library' };
const grants = [
  { token: 'A', display_name: '甲.md' },
  { token: 'B', display_name: '乙.pdf' },
  { token: 'C', display_name: '丙.txt' },
];
function controller(send) {
  let saved;
  return new SourceActions(
    'sources',
    identity,
    {
      read: async () => saved,
      put: async (value) => {
        assert.equal(saved, undefined);
        saved = structuredClone(value);
      },
      remove: async () => {
        saved = undefined;
      },
    },
    send,
  );
}
const resultsFor = (files) => (token, state, error) => {
  const item = files.find((value) => value.token === token);
  item.state = state;
  item.error = error;
};

test('a definitively rejected file is reported separately and remaining imports continue', async () => {
  const files = prepareSourceFiles(grants, 'collection-original');
  const sent = [];
  const actions = controller(async (intent) => {
    sent.push(intent.command);
    if (sent.length === 2)
      throw Object.assign(new Error('grant expired'), { details: { code: 'input_unavailable' } });
  });
  await actions.load();
  await submitSourceFiles(files, actions, resultsFor(files));
  assert.deepEqual(
    files.map((file) => file.state),
    ['accepted', 'failed', 'accepted'],
  );
  assert.equal(files[1].error, 'grant expired');
  assert.equal(sent.length, 3);
  assert.equal(actions.getSnapshot().pending, null);
});

test('unknown acceptance pauses; exact request confirmation then continues only untouched files', async () => {
  const files = prepareSourceFiles(grants, 'collection-original');
  const originals = structuredClone(files.map((file) => file.command));
  const sent = [];
  const actions = controller(async (intent) => {
    sent.push(intent.command);
    if (sent.length === 2) throw new Error('lost acknowledgement');
  });
  await actions.load();
  await submitSourceFiles(files, actions, resultsFor(files));
  assert.deepEqual(
    files.map((file) => file.state),
    ['accepted', 'unconfirmed', 'waiting'],
  );
  await submitSourceFiles(files, actions, resultsFor(files));
  assert.equal(sent.length, 2);
  await actions.retry();
  files[1].state = 'accepted';
  await submitSourceFiles(files, actions, resultsFor(files));
  assert.deepEqual(sent[1], sent[2]);
  assert.deepEqual(sent[3], originals[2]);
  assert.equal(sent[3].payload.request.collection_id, 'collection-original');
  assert.deepEqual(
    files.map((file) => file.state),
    ['accepted', 'accepted', 'accepted'],
  );
});

test('a new import starts a fresh batch instead of merging with the previous progress count', () => {
  const source = readFileSync(
    new URL('../src/sources/useSourceImports.ts', import.meta.url),
    'utf8',
  );
  assert.ok(source.includes('setItems(files)'), 'file and folder batches replace the shown batch');
  assert.ok(
    !source.includes('setItems((old) => [...old, ...files])'),
    'a new batch must not inherit rows from an earlier import',
  );
});

test('prepared filenames, format, collection and command IDs remain independent of later grants', () => {
  const current = structuredClone(grants);
  const files = prepareSourceFiles(current, 'collection-before', ['故障排查', 'CAN 总线']);
  current[0].display_name = 'later.pdf';
  current[0].token = 'later-token';
  assert.equal(files[0].command.payload.request.input_token, 'A');
  assert.equal(files[0].command.payload.request.original_name, '甲.md');
  assert.deepEqual(
    files.map((file) => file.command.payload.request.format),
    ['markdown', 'pdf', 'text'],
  );
  assert.ok(
    files.every((file) => file.command.payload.request.collection_id === 'collection-before'),
  );
  assert.deepEqual(files[0].command.payload.tag_names, ['故障排查', 'CAN 总线']);
  assert.equal(new Set(files.map((file) => file.command.payload.command_id)).size, 3);
});

test('an already occupied source controller leaves unsubmitted file choices unchanged', async () => {
  const files = prepareSourceFiles(grants, null);
  let sends = 0;
  const actions = controller(async () => {
    sends++;
    throw new Error('unknown prior request');
  });
  await actions.load();
  await actions.execute(
    {
      kind: 'sources_import_webpage',
      payload: {
        command_id: 'existing-webpage',
        request: {
          url: 'https://example.com/',
          collection_id: null,
          add_version_to: null,
          base_row_version: null,
        },
      },
    },
    'job_accepted',
  );
  await submitSourceFiles(files, actions, resultsFor(files));
  assert.equal(sends, 1);
  assert.ok(files.every((file) => file.state === 'waiting'));
});

test('A to B to A list scopes reject the original late page even when values match again', () => {
  const epoch = new SourceListEpoch();
  const originalA = epoch.begin();
  assert.equal(epoch.current(), originalA);
  const changedB = epoch.begin();
  const currentA = epoch.begin();
  assert.notEqual(currentA, originalA);
  assert.equal(epoch.isCurrent(originalA), false);
  assert.equal(epoch.isCurrent(changedB), false);
  assert.equal(epoch.isCurrent(currentA), true);
});

// The interface labels a grant before the import runs, so it mirrors the
// kernel's importable-extension list. That list is the one the temporary "open
// documents" reader, the directory scan and both file dialogs read, so a format
// missing here is a format the user can pick, drop or open and then watch fail
// when it is saved. Reading the kernel list keeps the two from drifting apart.
test('the interface mirrors every extension the kernel can import', () => {
  const source = readFileSync(
    new URL('../../../crates/pkms-app/src/source_imports.rs', import.meta.url),
    'utf8',
  );
  // rustfmt may wrap the declaration, so read from the constant to its terminator.
  const start = source.indexOf('pub const IMPORTABLE_EXTENSIONS');
  const end = start < 0 ? -1 : source.indexOf('];', start);
  assert.ok(start >= 0 && end > start, 'the kernel importable-extension list must exist');
  const line = source.slice(start, end);
  const kernel = [...line.matchAll(/"([a-z0-9]+)"/g)].map((match) => match[1]);
  assert.ok(kernel.length > 3, `expected the full extension list, saw ${kernel.length}`);
  assert.deepEqual(
    kernel.slice().sort(),
    Object.keys(SOURCE_FILE_FORMATS).sort(),
    'the interface must mirror the kernel importable-extension list',
  );
});
test('unsupported file grants are reported without submitting them as text', async () => {
  const files = prepareSourceFiles(
    [
      { token: 'image', display_name: 'diagram.png' },
      { token: 'pdf', display_name: 'Article.PDF' },
    ],
    null,
  );
  const sent = [];
  const actions = controller(async (intent) => {
    sent.push(intent.command);
  });
  await actions.load();
  await submitSourceFiles(files, actions, resultsFor(files));
  assert.equal(files[0].state, 'failed');
  assert.match(files[0].error, /Only PDF/);
  assert.equal(sent.length, 1);
  assert.equal(sent[0].payload.request.input_token, 'pdf');
});

test('batches larger than the recent task window retain each accepted task ID', async () => {
  const files = prepareSourceFiles(
    Array.from({ length: 25 }, (_, index) => ({
      token: 'token-' + index,
      display_name: 'Document-' + index + '.pdf',
    })),
    null,
  );
  const actions = controller(async (intent) => ({
    job_id: 'job-' + intent.command.payload.request.input_token,
  }));
  await actions.load();
  const results = [];
  await submitSourceFiles(files, actions, (token, state, error, jobId) =>
    results.push({ token, state, error, jobId }),
  );
  assert.equal(results.length, 25);
  assert.ok(
    results.every((item) => item.state === 'accepted' && item.jobId === 'job-' + item.token),
  );
  assert.equal(new Set(results.map((item) => item.jobId)).size, 25);
});
const scannedEntry = (over) => ({
  relative_path: 'a.md',
  name: 'a.md',
  size: 10,
  format: 'markdown',
  status: 'ready',
  note: null,
  existing_title: null,
  existing_trashed: false,
  grant: { token: 'g-a' },
  ...over,
});

test('a folder scan separates importable entries from reported skips and duplicates', () => {
  const { ready, duplicates, skipped, importable } = splitScannedEntries([
    scannedEntry({}),
    scannedEntry({
      relative_path: 'big.txt',
      name: 'big.txt',
      format: 'text',
      note: 'not_hashed',
      grant: { token: 'g-big' },
    }),
    scannedEntry({
      relative_path: 'copy.md',
      name: 'copy.md',
      status: 'duplicate',
      existing_title: 'original.md',
      grant: { token: 'g-copy' },
    }),
    scannedEntry({
      relative_path: 'image.png',
      name: 'image.png',
      format: null,
      status: 'unsupported',
      note: 'unsupported_format',
      grant: null,
    }),
    scannedEntry({
      relative_path: 'empty.txt',
      name: 'empty.txt',
      format: 'text',
      size: 0,
      status: 'empty',
      note: 'empty_file',
      grant: null,
    }),
    scannedEntry({
      relative_path: 'huge.pdf',
      name: 'huge.pdf',
      format: 'pdf',
      status: 'too_large',
      note: 'too_large',
      grant: null,
    }),
    scannedEntry({
      relative_path: 'late.txt',
      name: 'late.txt',
      format: 'text',
      status: 'limit_reached',
      note: 'limit_reached',
      grant: null,
    }),
  ]);
  assert.deepEqual(
    ready.map((entry) => entry.relative_path),
    ['a.md', 'big.txt'],
  );
  assert.deepEqual(
    duplicates.map((entry) => entry.relative_path),
    ['copy.md'],
  );
  assert.deepEqual(
    importable.map((entry) => entry.relative_path),
    ['a.md', 'big.txt', 'copy.md'],
  );
  assert.deepEqual(
    skipped.map((entry) => entry.status),
    ['unsupported', 'empty', 'too_large', 'limit_reached'],
  );
  assert.equal(skipped[2].note, 'too_large');
});

test('scanned entries are submitted with their own token, name and format', () => {
  const entries = [
    scannedEntry({}),
    scannedEntry({
      relative_path: 'copy.md',
      name: 'copy.md',
      status: 'duplicate',
      existing_title: 'original.md',
      grant: { token: 'g-copy' },
    }),
  ];
  const files = prepareScannedEntries(splitScannedEntries(entries).importable, 'collection-x', [
    '阅读摘录',
  ]);
  assert.deepEqual(
    files.map((file) => file.token),
    ['g-a', 'g-copy'],
  );
  assert.deepEqual(
    files.map((file) => file.command.payload.request.format),
    ['markdown', 'markdown'],
  );
  assert.equal(files[0].command.payload.request.original_name, 'a.md');
  assert.equal(files[0].command.payload.request.collection_id, 'collection-x');
  assert.deepEqual(files[0].command.payload.tag_names, ['阅读摘录']);
  assert.equal(files[0].state, 'waiting');
  assert.equal(new Set(files.map((file) => file.command.payload.command_id)).size, files.length);
});

test('a cancelled batch stops submitting and leaves untouched items waiting', async () => {
  const files = prepareSourceFiles(grants, null);
  const sent = [];
  let stop = false;
  const actions = controller(async (intent) => {
    sent.push(intent.command);
    stop = true;
  });
  await actions.load();
  await submitSourceFiles(files, actions, resultsFor(files), () => stop);
  assert.equal(sent.length, 1);
  assert.deepEqual(
    files.map((file) => file.state),
    ['accepted', 'waiting', 'waiting'],
  );
});

test('skip wording comes from the UI, never from a machine code', () => {
  const t = (zh) => zh;
  assert.equal(scanSkipReason('unsupported', t), '不支持的类型');
  assert.equal(scanSkipReason('empty', t), '空文件');
  assert.equal(scanSkipReason('too_large', t), '超过 256 MiB');
  assert.equal(scanSkipReason('unreadable', t), '无法读取');
  assert.equal(scanSkipReason('limit_reached', t), '超过单次受理上限');
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  failedSourceJobs,
  sourceBatchProgress,
  sourceBatchProgressLabel,
  sourceBatchProgressParts,
  sourceImportFailures,
} from '../src/sources/source-import-batch.ts';

const join = (zh, en) => `${zh}\u0001${en}`;
const zh = (zhText) => zhText;
const en = (_, enText) => enText;

const item = (token, state, jobId) => ({ token, name: `${token}.pdf`, state, jobId });
const job = (id, status, errorCode, inputToken, stage = 'indexed') => ({
  id,
  status,
  stage,
  error_code: errorCode,
  input: { input_token: inputToken ?? id },
});

// Before this count existed the sources page only said "正在导入资料…" and the
// activity panel only listed rows, so a 500-file batch had no total and no
// completed count anywhere on screen.
test('a batch reports how many of its items are done', () => {
  const items = [
    item('a', 'accepted', 'j-a'),
    item('b', 'accepted', 'j-b'),
    item('c', 'accepted', 'j-c'),
    item('d', 'accepted', 'j-d'),
    item('e', 'waiting'),
  ];
  const jobs = [
    job('j-a', 'succeeded', null, 'a'),
    job('j-b', 'succeeded', null, 'b', 'existing_source'),
    job('j-c', 'failed', 'worker_unavailable', 'c'),
    job('j-d', 'running', null, 'd'),
  ];
  const progress = sourceBatchProgress(items, jobs);
  assert.deepEqual(progress, {
    total: 5,
    finished: 3,
    duplicates: 1,
    succeeded: 1,
    failed: 1,
    canceled: 0,
    active: 1,
    waiting: 1,
    percent: 60,
  });
  assert.equal(sourceBatchProgressLabel(progress, zh), '已完成 3 / 5 份');
  assert.equal(sourceBatchProgressLabel(progress, en), '3 of 5 done');
  assert.deepEqual(sourceBatchProgressParts(progress, zh), [
    '新增成功 1 份',
    '重复归并 1 份',
    '失败 1 份',
    '进行中 1 份',
    '等待 1 份',
  ]);
  assert.deepEqual(sourceBatchProgressParts(progress, en), [
    '1 new',
    '1 already in library',
    '1 failed',
    '1 in progress',
    '1 waiting',
  ]);
});

test('an accepted item whose receipt has not arrived is not counted as done', () => {
  const items = [item('a', 'accepted', 'j-a'), item('b', 'accepted', 'j-b')];
  const progress = sourceBatchProgress(items, []);
  assert.equal(progress.finished, 0);
  assert.equal(progress.percent, 0);
  assert.equal(progress.active, 2);
  // The row states the batch is queued, not that it finished.
  assert.equal(progress.succeeded, 0);
});

test('a rejected submission and a cancelled job are finished, in separate buckets', () => {
  const items = [item('a', 'failed'), item('b', 'accepted', 'j-b'), item('c', 'unconfirmed')];
  const jobs = [job('j-b', 'canceled', 'canceled', 'b')];
  const progress = sourceBatchProgress(items, jobs);
  assert.deepEqual(progress, {
    total: 3,
    finished: 2,
    duplicates: 0,
    succeeded: 0,
    failed: 1,
    canceled: 1,
    active: 0,
    waiting: 1,
    percent: 67,
  });
  // Cancelling on purpose is not a failure; the wording stays separate.
  assert.deepEqual(sourceBatchProgressParts(progress, zh), [
    '失败 1 份',
    '已取消 1 份',
    '等待 1 份',
  ]);
});

test('an empty batch has an empty label instead of "0 / 0"', () => {
  const progress = sourceBatchProgress([], []);
  assert.equal(progress.total, 0);
  assert.equal(progress.percent, 0);
  assert.equal(progress.duplicates, 0);
  assert.equal(sourceBatchProgressLabel(progress, zh), '');
  assert.deepEqual(sourceBatchProgressParts(progress, zh), []);
});

test('failures keep the code that explains them, including cancellations', () => {
  const items = [
    item('a', 'accepted', 'j-a'),
    item('b', 'accepted', 'j-b'),
    item('c', 'failed'),
    item('d', 'waiting'),
    item('e', 'unconfirmed'),
    item('f', 'accepted', 'j-f'),
  ];
  const jobs = [
    job('j-a', 'failed', 'pdf_invalid', 'a'),
    job('j-b', 'canceled', 'canceled', 'b'),
    job('j-f', 'running', null, 'f'),
  ];
  assert.deepEqual(sourceImportFailures(items, jobs), [
    { token: 'a', name: 'a.pdf', code: 'pdf_invalid' },
    { token: 'b', name: 'b.pdf', code: 'canceled' },
    // A submission the kernel rejected has no code of its own; the row's message
    // is all the panel can offer, so the code stays null instead of being invented.
    { token: 'c', name: 'c.pdf', code: null },
  ]);
});

test('only failed and cancelled jobs join the grouped list', () => {
  const jobs = [
    job('run', 'running', null),
    job('queued', 'queued', null),
    job('ok', 'succeeded', null),
    job('bad', 'failed', 'docx_invalid_xml'),
    job('gone', 'canceled', 'canceled'),
  ];
  assert.deepEqual(
    failedSourceJobs(jobs).map((entry) => entry.id),
    ['bad', 'gone'],
  );
});

// The dialog can be closed while a batch keeps running, so the sources page has to
// carry the count itself. The panel render test asserts the same pure function on
// real markup; this pass only locks the page wiring that the assertion cannot see.
test('the sources page states the completed count while the dialog is closed', () => {
  const source = readFileSync(new URL('../src/sources/SourcesPage.tsx', import.meta.url), 'utf8');
  assert.ok(
    source.includes('sourceBatchProgress(imports.items, imports.jobs)'),
    'the page must count the batch it is showing',
  );
  assert.ok(
    source.includes('sourceBatchProgressLabel(batchProgress, t)'),
    'the page must render the completed count',
  );
});

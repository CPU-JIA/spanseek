import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SOURCE_IMPORT_FAILURE_CODES,
  SOURCE_IMPORT_FAILURE_GROUPS,
  groupImportFailures,
  importFailureSummary,
  sourceImportFailureGroup,
  sourceImportFailureGroupLabel,
} from '../src/sources/source-import-failure.ts';

const zh = (zhText) => zhText;
const en = (_, enText) => enText;

// The panel has to bundle ten causes from one batch — a missing component, a
// damaged file, a timeout, a cancel — without asking the user to read ten
// sentences to count them. The grouping is derived from the same table as the
// wording, so a code cannot gain a sentence and lose its group.
test('every code with wording also carries the group the panel bundles it under', () => {
  assert.ok(SOURCE_IMPORT_FAILURE_CODES.length >= 60);
  for (const code of SOURCE_IMPORT_FAILURE_CODES) {
    const group = sourceImportFailureGroup(code);
    assert.ok(SOURCE_IMPORT_FAILURE_GROUPS.includes(group), `${code} needs a known group`);
    if (code !== 'source_import_failed') {
      assert.notEqual(group, 'other', `${code} must not fall into the unknown group`);
    }
  }
  assert.equal(SOURCE_IMPORT_FAILURE_GROUPS.at(-1), 'other', 'unknown codes read last');
});

test('the user-facing causes stay apart', () => {
  const cases = {
    worker_unavailable: 'component',
    pdfium_unavailable: 'component',
    pdf_parse_failed: 'parser',
    docx_invalid_xml: 'parser',
    mobi_asset_limit: 'size',
    invalid_utf8: 'parser',
    webpage_timeout: 'timeout',
    worker_timeout: 'timeout',
    docx_input_limit: 'size',
    doc_structure_limit: 'size',
    xlsx_structure_limit: 'size',
    xls_structure_limit: 'size',
    pptx_structure_limit: 'size',
    webpage_size_limit: 'size',
    webpage_network_failed: 'network',
    webpage_invalid_url: 'network',
    webpage_login_required: 'content',
    html_no_content: 'content',
    input_unavailable: 'local',
    storage_full: 'local',
    canceled: 'canceled',
  };
  for (const [code, expected] of Object.entries(cases)) {
    assert.equal(sourceImportFailureGroup(code), expected, code);
  }
  // A job the user cancelled is not a failure the user has to repair.
  assert.notEqual(sourceImportFailureGroup('canceled'), sourceImportFailureGroup('pdf_invalid'));
});

test('a code this build does not know is reported as unknown, never dropped', () => {
  assert.equal(sourceImportFailureGroup('a_future_code_from_a_newer_core'), 'other');
  assert.equal(sourceImportFailureGroup(null), 'other');
  assert.equal(sourceImportFailureGroup(undefined), 'other');
  // Prototype keys must not be mistaken for mapped codes.
  assert.equal(sourceImportFailureGroup('constructor'), 'other');
});

test('records bundle in a fixed order and keep their own order inside a group', () => {
  const records = [
    { id: 'cancel', code: 'canceled' },
    { id: 'damaged-2', code: 'docx_invalid_xml' },
    { id: 'missing-component', code: 'worker_unavailable' },
    { id: 'damaged-1', code: 'pdf_invalid' },
    { id: 'unknown', code: 'brand_new_code' },
    { id: 'too-big', code: 'size_limit' },
  ];
  const groups = groupImportFailures(records, (record) => sourceImportFailureGroup(record.code));
  assert.deepEqual(
    groups.map((group) => group.group),
    ['component', 'parser', 'size', 'canceled', 'other'],
  );
  assert.deepEqual(
    groups.map((group) => group.count),
    [1, 2, 1, 1, 1],
  );
  assert.deepEqual(
    groups.find((group) => group.group === 'parser').records.map((record) => record.id),
    ['damaged-2', 'damaged-1'],
  );
});

test('the summary reads as one line in both languages, with a singular English form', () => {
  const groups = [
    { group: 'component', count: 2 },
    { group: 'canceled', count: 1 },
  ];
  assert.equal(
    importFailureSummary(groups, (zhText) => zhText),
    '3 项未完成：安装或组件问题 ×2 · 已取消 ×1',
  );
  assert.equal(
    importFailureSummary(groups, (_, enText) => enText),
    '3 items did not complete: Installation or component problem ×2 · Cancelled ×1',
  );
  assert.equal(
    importFailureSummary([{ group: 'timeout', count: 1 }], (_, enText) => enText),
    '1 item did not complete: Timed out ×1',
  );
  // Nothing failed: the caller can render the line unconditionally.
  assert.equal(importFailureSummary([], zh), '');
});

test('every group has its own heading in both languages', () => {
  const seen = new Map();
  for (const group of SOURCE_IMPORT_FAILURE_GROUPS) {
    const labelZh = sourceImportFailureGroupLabel(group, zh);
    const labelEn = sourceImportFailureGroupLabel(group, en);
    assert.ok(labelZh.trim().length > 0, `${group} needs Chinese wording`);
    assert.ok(labelEn.trim().length > 0, `${group} needs English wording`);
    assert.notEqual(labelZh, labelEn, `${group} must actually be translated`);
    assert.ok(!seen.has(labelZh), `${group} repeats ${seen.get(labelZh)}`);
    seen.set(labelZh, group);
  }
});

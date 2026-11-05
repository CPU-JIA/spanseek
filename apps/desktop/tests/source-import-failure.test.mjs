import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SOURCE_IMPORT_FAILURE_CODES,
  parseComponentNotice,
  sourceImportFailure,
} from '../src/sources/source-import-failure.ts';

// The core reports machine codes, so both languages are asserted in one pass.
const join = (zh, en) => `${zh}\u0001${en}`;
const split = (text) => text.split('\u0001');

const FALLBACK_ZH = '读取资料失败，可在任务详情查看原因或重试。';
const FALLBACK_EN = 'The document could not be read. See activity details or retry.';
const UNKNOWN_CODE = 'a_future_code_from_a_newer_core';

test('the fallback sentence is reserved for codes this build does not know', () => {
  const [zh, en] = split(sourceImportFailure(UNKNOWN_CODE, join));
  assert.equal(zh, FALLBACK_ZH);
  assert.equal(en, FALLBACK_EN);
});

test('every code on the import chain reads as prose instead of the fallback', () => {
  assert.ok(SOURCE_IMPORT_FAILURE_CODES.length >= 30);
  const fallback = sourceImportFailure(UNKNOWN_CODE, join);
  for (const code of SOURCE_IMPORT_FAILURE_CODES) {
    const text = sourceImportFailure(code, join);
    assert.notEqual(text, fallback, `${code} must not collapse into the fallback`);
    const [zh, en] = split(text);
    assert.ok(zh.trim().length > 0, `${code} needs Chinese text`);
    assert.ok(en.trim().length > 0, `${code} needs English text`);
  }
});

test('distinct causes keep distinct wording', () => {
  const cases = [
    'webpage_timeout',
    'webpage_login_required',
    'docx_input_limit',
    'doc_structure_limit',
    'xlsx_structure_limit',
    'xls_structure_limit',
    'pptx_structure_limit',
    'pdfium_unavailable',
    'html_no_content',
    'webpage_dynamic_content',
    'canceled',
  ];
  const texts = cases.map((code) => sourceImportFailure(code, join));
  assert.equal(new Set(texts).size, cases.length);
});
test('an unavailable import component reads as an installation fix, not a bare code', () => {
  // Before the task list used this mapping it rendered `worker_unavailable` and
  // offered a retry that could never repair the installation.
  const [zh, en] = split(sourceImportFailure('worker_unavailable', join));
  assert.notEqual(zh, 'worker_unavailable');
  assert.ok(!zh.includes('worker_unavailable'), zh);
  assert.ok(zh.includes('spanseek-worker.exe'), zh);
  assert.ok(zh.includes('重新安装'), zh);
  assert.ok(en.includes('spanseek-worker.exe'), en);
  assert.ok(en.includes('installation'), en);
});

// `parseComponentNotice` translates twice: its own line and the shared sentence it
// reuses. This pass keeps both languages composable instead of tuple-encoding them.
const compose = (zh, en) => `zh:${zh}\nen:${en}`;

test('the parse component report names the missing path and the next step', () => {
  const path = 'C:\\spanseek\\spanseek-worker.exe';
  const line = parseComponentNotice({ code: 'worker_unavailable', path }, compose);
  assert.ok(line.includes(path), line);
  assert.ok(line.includes('重新安装'), line);
  assert.ok(line.includes('reinstall spanseek'), line);
  // A component with no configured path still reads as prose, never as `null`.
  const unconfigured = parseComponentNotice({ code: 'pdfium_unavailable', path: null }, compose);
  assert.ok(unconfigured.includes('PDF 解析组件不可用'), unconfigured);
  assert.ok(!unconfigured.includes('null'), unconfigured);
});

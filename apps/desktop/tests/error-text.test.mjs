import test from 'node:test';
import assert from 'node:assert/strict';
import {
  apiErrorMessage,
  apiErrorRetryable,
  errorText,
  operationErrorMessage,
} from '../src/errors.ts';

const CODES = [
  'backend_starting',
  'query_expired',
  'invalid_input',
  'invalid_document',
  'unsupported_api',
  'instance_mismatch',
  'app_state_conflict',
  'revision_conflict',
  'metadata_conflict',
  'idempotency_mismatch',
  'not_found',
  'object_trashed',
  'storage_busy',
  'storage_full',
  'io_failed',
  'integrity_failed',
  'unsupported_database',
  'migration_failed',
  'input_unavailable',
  'output_exists',
  'internal',
];

test('every kernel error code has its own text in both languages', () => {
  const zh = new Set();
  const en = new Set();
  for (const code of CODES) {
    const chinese = apiErrorMessage(code, 'zh-CN');
    const english = apiErrorMessage(code, 'en');
    assert.ok(chinese.length > 0, code);
    assert.ok(english.length > 0, code);
    assert.notEqual(chinese, english, code);
    zh.add(chinese);
    en.add(english);
  }
  assert.equal(zh.size, CODES.length);
  assert.equal(en.size, CODES.length);
});

test('an unsupported workspace explains that the data was left alone', () => {
  assert.match(apiErrorMessage('unsupported_database', 'zh-CN'), /原数据未被替换/);
  assert.match(apiErrorMessage('unsupported_database', 'en'), /left unchanged/i);
  assert.match(apiErrorMessage('unsupported_database', 'zh-CN'), /更新 spanseek/);
  assert.match(apiErrorMessage('unsupported_database', 'en'), /update spanseek/i);
  assert.match(apiErrorMessage('migration_failed', 'en'), /snapshot/i);
});

test('retryability matches the kernel table', () => {
  assert.equal(apiErrorRetryable('backend_starting'), true);
  assert.equal(apiErrorRetryable('query_expired'), true);
  assert.equal(apiErrorRetryable('storage_busy'), true);
  assert.equal(apiErrorRetryable('storage_full'), true);
  assert.equal(apiErrorRetryable('io_failed'), true);
  assert.equal(apiErrorRetryable('unsupported_database'), false);
  assert.equal(apiErrorRetryable('integrity_failed'), false);
  assert.equal(apiErrorRetryable('invalid_input'), false);
  assert.equal(apiErrorRetryable('output_exists'), false);
});

test('operation failures are distinguishable instead of one generic sentence', () => {
  const codes = [
    'input_unavailable',
    'integrity_failed',
    'storage_full',
    'io_failed',
    'canceled',
    'pdf_renderer_unavailable',
    'pdf_render_failed',
    'pdf_render_timeout',
  ];
  const seen = new Set(codes.map((code) => operationErrorMessage(code, 'zh-CN')));
  assert.equal(seen.size, codes.length);
  for (const code of codes) {
    assert.notEqual(
      operationErrorMessage(code, 'zh-CN'),
      operationErrorMessage('unknown_code', 'zh-CN'),
      code,
    );
  }
  // An unknown code keeps the generic sentence rather than inventing a reason.
  assert.equal(operationErrorMessage('unknown_code', 'en'), operationErrorMessage(null, 'en'));
});
test('a restored copy explains that unfinished tasks do not resume', () => {
  const zh = operationErrorMessage('restored_copy', 'zh-CN');
  const en = operationErrorMessage('restored_copy', 'en');
  assert.match(zh, /备份快照/);
  assert.match(zh, /不会在恢复副本中继续/);
  assert.match(en, /backup/i);
  assert.match(en, /will not continue in the restored copy/i);
});
test('a damaged workspace is not described as a damaged backup', () => {
  const zh = operationErrorMessage('integrity_failed', 'zh-CN', 'open_workspace');
  const en = operationErrorMessage('integrity_failed', 'en', 'create_workspace');
  assert.doesNotMatch(zh, /备份未通过/);
  assert.match(zh, /知识库/);
  assert.match(zh, /完整备份恢复/);
  assert.match(en, /workspace/i);
  assert.doesNotMatch(en, /choose another complete backup/i);
  // Backup operations keep the backup-specific recovery step.
  assert.match(operationErrorMessage('integrity_failed', 'zh-CN', 'restore_backup'), /备份未通过/);
  assert.match(operationErrorMessage('integrity_failed', 'en', 'verify_backup'), /backup/i);
  // Without a kind the shared sentence stays exactly as before.
  assert.equal(
    operationErrorMessage('integrity_failed', 'zh-CN'),
    '备份未通过完整性校验。请保留原文件，并选择另一份完整备份。',
  );
});
test('CSL failures keep their actionable reason in both languages', () => {
  const failure = (reason) => ({
    details: {
      code: 'invalid_input',
      message: 'generic',
      request_id: 'r',
      retryable: false,
      details: { kind: 'citation_failure', reason },
    },
  });
  assert.match(errorText(failure('csl_invalid_style'), 'zh-CN'), /CSL 样式格式无效/);
  assert.match(errorText(failure('csl_parent_missing'), 'en'), /parent is missing/i);
  assert.match(errorText(failure('csl_style_in_use'), 'zh-CN'), /仍被依赖样式使用/);
  assert.match(errorText(failure('csl_style_in_use'), 'en'), /dependent style/i);
  assert.equal(errorText(failure('future_reason'), 'en'), apiErrorMessage('invalid_input', 'en'));
});

test('sync failures distinguish viewer, revoked and connection states', () => {
  const failure = (reason) => ({
    details: {
      code: 'invalid_input',
      message: 'generic',
      request_id: 'r',
      retryable: false,
      details: { kind: 'sync_failure', reason },
    },
  });
  assert.match(errorText(failure('insufficient_role'), 'zh-CN'), /只读者/);
  assert.match(errorText(failure('insufficient_role'), 'en'), /viewer/i);
  assert.match(errorText(failure('device_revoked'), 'zh-CN'), /已被所有者移除/);
  assert.match(errorText(failure('device_revoked'), 'en'), /revoked/i);
  assert.match(errorText(failure('sync_failed'), 'zh-CN'), /本地内容已保留/);
  assert.match(errorText(failure('sync_failed'), 'en'), /local content was kept/i);
});

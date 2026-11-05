import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { inferenceErrorMessage } from '../src/inference/inference-errors.ts';
const failure = (reason) =>
  Object.assign(Error('untrusted upstream body: secret-fixture'), {
    details: { details: { kind: 'inference_failure', reason } },
  });
test('remote failures use translated controlled descriptions without exposing upstream error bodies', () => {
  for (const reason of [
    'model_key_missing',
    'model_invalid_key',
    'model_credential_unavailable',
    'model_credential_unsupported',
    'model_authentication_failed',
    'model_rate_limited',
    'model_api_error',
    'model_invalid_response',
    'model_response_too_large',
    'model_incomplete',
    'model_unsupported_output',
    'model_invalid_request',
    'model_invalid_endpoint',
    'model_unavailable',
    'unknown-upstream-code',
  ]) {
    const zh = inferenceErrorMessage(failure(reason), (zh) => zh);
    const en = inferenceErrorMessage(failure(reason), (_, en) => en);
    assert.notEqual(zh, en);
    assert.equal(zh.includes('secret-fixture'), false);
    assert.equal(en.includes('secret-fixture'), false);
    assert.ok(zh.length > 5);
    assert.ok(en.length > 5);
  }
});
test('local missing-model guidance remains separate from generic service availability', () => {
  const unavailable = inferenceErrorMessage(failure('model_unavailable'), (_, en) => en);
  const missing = inferenceErrorMessage(failure('model_not_found'), (_, en) => en);
  assert.equal(unavailable.includes('download'), false);
  assert.match(missing, /download/);
});

test('interrupted chat explains retained partial text and no automatic model replay', () => {
  const text = inferenceErrorMessage('worker_interrupted', (_, en) => en);
  assert.match(text, /Generated text is retained/);
  assert.match(text, /not be called again automatically/);
});

test('interrupted strict answers explain the unknown outcome without suggesting automatic replay', () => {
  for (const error of ['interrupted_unknown_outcome', failure('interrupted_unknown_outcome')]) {
    const en = inferenceErrorMessage(error, (_, en) => en);
    const zh = inferenceErrorMessage(error, (zh) => zh);
    assert.match(en, /final outcome is unknown/);
    assert.match(en, /not be called again automatically/);
    assert.match(en, /Start a new question/);
    assert.match(zh, /最终结果未知/);
    assert.match(zh, /不会自动重新调用模型/);
    assert.equal(en.includes('secret-fixture'), false);
    assert.equal(zh.includes('interrupted_unknown_outcome'), false);
  }
});

test('embedding availability explains missing setup and credentials without raw codes', () => {
  for (const reason of [
    'embedding_unconfigured',
    'index_unavailable',
    'model_key_missing',
    'embedding_remote_required',
  ]) {
    const text = inferenceErrorMessage(reason, (_, en) => en);
    assert.equal(text.includes(reason), false);
    assert.match(text, /index|BGE-M3|key/i);
  }
  assert.match(
    inferenceErrorMessage('model_key_missing', (_, en) => en),
    /No API key is saved/,
  );
  assert.match(
    inferenceErrorMessage('model_credential_unavailable', (_, en) => en),
    /storage is unavailable/,
  );
});

// The stored code is the contract between the kernel and the interface: a job
// row renders it directly, and an IPC failure carries it as the reason. Every
// code the worker can store must therefore have its own controlled sentence,
// otherwise a real failure is shown as a generic "the operation did not
// complete". The table is read from the kernel so a new code cannot ship
// without its wording.
test('every stable inference failure code has its own controlled description', () => {
  const source = readFileSync(
    new URL('../../../crates/pkms-app/src/inference_process.rs', import.meta.url),
    'utf8',
  );
  const start = source.indexOf('pub(crate) fn failure_code');
  assert.notEqual(start, -1, 'the kernel failure-code table must exist');
  const table = source.slice(start, source.indexOf('\n}', start));
  const codes = [...table.matchAll(/"([a-z0-9_]+)"\s*=>\s*"\1"/g)].map((match) => match[1]);
  assert.ok(codes.length > 40, `expected the full failure-code table, saw ${codes.length}`);
  const generic = [
    ['操作未完成，请重试。', 'The operation did not complete. Retry.'],
    [
      '模型请求未完成，请检查配置后重试。',
      'The model request did not complete. Check the configuration and retry.',
    ],
  ];
  for (const code of codes) {
    for (const error of [code, failure(code)]) {
      const zh = inferenceErrorMessage(error, (zh) => zh);
      const en = inferenceErrorMessage(error, (_, en) => en);
      for (const [zhText, enText] of generic) {
        assert.notEqual(zh, zhText, `${code} fell back to a generic sentence`);
        assert.notEqual(en, enText, `${code} fell back to a generic sentence`);
      }
      assert.ok(zh.length > 5, `${code} needs Chinese wording`);
      assert.ok(en.length > 5, `${code} needs English wording`);
      assert.equal(zh.includes('secret-fixture'), false);
      assert.equal(en.includes('secret-fixture'), false);
    }
  }
});

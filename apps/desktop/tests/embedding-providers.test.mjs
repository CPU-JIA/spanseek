import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DIMENSION_CHOICES,
  EMBEDDING_PROVIDERS,
  modelOption,
  plainHttpAllowed,
  providerLabel,
  providerOption,
} from '../src/inference/embedding-providers.ts';

function model(id, name) {
  const option = modelOption(id, name);
  assert.ok(option, `${id} must offer ${name}`);
  return option;
}

test('every provider keeps its documented endpoint and verified models', () => {
  assert.deepEqual(
    EMBEDDING_PROVIDERS.map((option) => option.id),
    ['cloudflare', 'openai', 'gemini', 'openai_compatible'],
  );
  assert.equal(providerOption('cloudflare').endpoint, '');
  assert.equal(
    providerOption('cloudflare').endpointHint,
    'https://api.cloudflare.com/client/v4/accounts/<ACCOUNT_ID>/ai/v1',
  );
  assert.equal(model('cloudflare', '@cf/baai/bge-m3').dimensions, 1024);
  assert.equal(model('cloudflare', '@cf/baai/bge-m3').price, '$0.012');
  assert.equal(providerOption('openai').endpoint, 'https://api.openai.com/v1');
  assert.equal(model('openai', 'text-embedding-3-small').dimensions, 1536);
  assert.equal(model('openai', 'text-embedding-3-large').dimensions, 3072);
  assert.equal(model('openai', 'text-embedding-3-large').inputTokens, 8192);
  assert.equal(
    providerOption('gemini').endpoint,
    'https://generativelanguage.googleapis.com/v1beta',
  );
  assert.equal(providerOption('openai_compatible').freeModel, true);
  assert.equal(providerLabel('openai_compatible', false), 'OpenAI 兼容端点（自建/网关）');
  assert.equal(DIMENSION_CHOICES.includes(768), true);
});

test('a shut down model stays visible but cannot be chosen', () => {
  // text-embedding-004 was shut down on 2026-01-14; the list marks it retired.
  assert.equal(model('gemini', 'text-embedding-004').retired, true);
  assert.equal(model('gemini', 'gemini-embedding-001').retired, false);
  assert.equal(model('gemini', 'gemini-embedding-001').inputTokens, 2048);
  assert.equal(model('gemini', 'gemini-embedding-2').inputTokens, 8192);
  // Only the models that accept taskType may be told how the text is used.
  assert.equal(model('gemini', 'gemini-embedding-001').taskType, true);
  assert.equal(model('gemini', 'gemini-embedding-2').taskType, false);
});

test('plain http is accepted only for a loopback embedding server', () => {
  assert.equal(plainHttpAllowed('http://127.0.0.1:11434/v1'), true);
  assert.equal(plainHttpAllowed('http://localhost:1234/v1'), true);
  assert.equal(plainHttpAllowed('http://[::1]:8080/v1'), true);
  assert.equal(plainHttpAllowed('http://example.com/v1'), false);
  assert.equal(plainHttpAllowed('http://192.168.1.9:11434/v1'), false);
  assert.equal(plainHttpAllowed('https://api.openai.com/v1'), true);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { defaultTimeoutSeconds } from '../src/inference/generation-timeouts.ts';
import { agentDefaults } from '../src/inference/agent-state.ts';

test('the interface default matches the core table for every provider and purpose', () => {
  assert.equal(defaultTimeoutSeconds('ollama', 'answer'), 600);
  assert.equal(defaultTimeoutSeconds('ollama', 'chat'), 900);
  assert.equal(defaultTimeoutSeconds('ollama', 'embedding'), 900);
  assert.equal(defaultTimeoutSeconds('openai_chat', 'answer'), 180);
  assert.equal(defaultTimeoutSeconds('gemini', 'chat'), 300);
  assert.equal(defaultTimeoutSeconds('cloudflare', 'embedding'), 120);
  // An unknown or not-yet-chosen provider must not inherit the local budget.
  assert.equal(defaultTimeoutSeconds(null, 'answer'), 180);
  assert.equal(defaultTimeoutSeconds(undefined, 'chat'), 300);
});

test('every resolved default stays inside the run-option bounds', () => {
  for (const provider of ['ollama', 'openai_chat', 'anthropic', 'gemini', 'cloudflare', null]) {
    for (const purpose of ['answer', 'chat', 'embedding']) {
      const value = defaultTimeoutSeconds(provider, purpose);
      assert.ok(Number.isInteger(value) && value >= 1 && value <= 3600, `${provider} ${purpose}`);
    }
  }
});

test('agent defaults carry the provider budget instead of a fixed 240', () => {
  assert.equal(agentDefaults('ollama').total_timeout_seconds, 600);
  assert.equal(agentDefaults('openai_chat').total_timeout_seconds, 180);
  assert.equal(agentDefaults(null).total_timeout_seconds, 180);
  assert.deepEqual(
    { ...agentDefaults('ollama'), total_timeout_seconds: 0 },
    { max_model_calls: 4, max_tool_calls: 8, max_evidence: 32, total_timeout_seconds: 0 },
  );
});

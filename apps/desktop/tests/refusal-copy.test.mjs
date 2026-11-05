import assert from 'node:assert/strict';
import test from 'node:test';
import { REFUSAL_MARKER, refusalCopy } from '../src/inference/refusal-copy.ts';

test('the refusal copy is fixed, authored in both languages and never the marker', () => {
  const zh = refusalCopy(false);
  const en = refusalCopy(true);
  // FA4-07 asks for fixed, predictable wording, so the copy is pinned here:
  // changing what a reader sees must be a deliberate edit of this test too.
  assert.deepEqual(zh, {
    label: '未找到可支撑结论的资料',
    body: '已在你授权的范围内完成检索，但库内资料不足以支撑一个可追溯的回答。为避免编造，本次不给出结论，也不引用任何未经核对的来源。',
    steps: [
      '导入或补充相关资料后重新提问。',
      '如果当前限定了范围或标签，扩大范围后再检索一次。',
      '换一种问法，或改问资料中明确写到的内容。',
    ],
  });
  assert.deepEqual(en, {
    label: 'No source in this library supports an answer',
    body: 'The search covered your authorised scope, but the library does not hold enough evidence for a traceable answer. Nothing was invented: no conclusion is given, and no unchecked source is cited.',
    steps: [
      'Import or add the relevant source, then ask again.',
      'If a scope or tag is applied, widen it and search once more.',
      'Rephrase the question, or ask about something the sources state explicitly.',
    ],
  });
});

test('both languages carry the same shape and the same guarantees', () => {
  for (const copy of [refusalCopy(false), refusalCopy(true)]) {
    assert.ok(copy.label.length > 0);
    assert.ok(copy.body.length > 0);
    assert.equal(copy.steps.length, 3);
    for (const step of copy.steps) assert.ok(step.length > 0);
    // The marker is machine state; a reader must never see it.
    assert.equal(JSON.stringify(copy).includes(REFUSAL_MARKER), false);
    assert.equal(JSON.stringify(copy).includes('[[PKMS'), false);
  }
  // The two languages are separately authored, not one copied into the other.
  assert.notDeepEqual(refusalCopy(false), refusalCopy(true));
  assert.notEqual(refusalCopy(false).body, refusalCopy(true).body);
  assert.notDeepEqual(refusalCopy(false).steps, refusalCopy(true).steps);
});

test('the machine marker stays byte-identical to the core constant', () => {
  // pkms_core::INSUFFICIENT_EVIDENCE_MARKER is the authority; this literal is
  // the cross-language contract that makes a refusal machine-detectable.
  assert.equal(REFUSAL_MARKER, '[[PKMS_INSUFFICIENT_EVIDENCE]]');
});

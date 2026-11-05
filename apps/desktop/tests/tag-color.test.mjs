import test from 'node:test';
import assert from 'node:assert/strict';
import { TAG_TONE_COUNT, tagTone, tagToneClass } from '../src/ui/tag-color.ts';

test('tag tone is stable for the same immutable id', () => {
  const id = '58379fc5-6f4d-47e8-a845-9b34b17b9c36';
  assert.equal(tagTone(id), tagTone(id));
  assert.match(tagToneClass(id), /^tag-tone tag-tone-(?:[0-9]|1[01])$/);
});

test('tag tone distribution uses the full palette', () => {
  const tones = new Set(
    Array.from({ length: 240 }, (_, index) =>
      tagTone(`00000000-0000-4000-8000-${index.toString(16).padStart(12, '0')}`),
    ),
  );
  assert.equal(tones.size, TAG_TONE_COUNT);
});

test('tag tone ignores display-name changes because it only accepts the id', () => {
  const id = '4e9b2141-6a65-4b5f-8677-b94b21651a0a';
  const before = tagToneClass(id);
  const after = tagToneClass(id);
  assert.equal(before, after);
  assert.equal(before, tagToneClass(id));
});

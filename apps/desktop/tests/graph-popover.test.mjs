import test from 'node:test';
import assert from 'node:assert/strict';
import { placeGraphPopover } from '../src/organization/graph-popover.ts';

const anchor = (over = {}) => ({ left: 296, top: 138, right: 351, bottom: 170, ...over });

test('a tag popover below the anchor stays inside the viewport', () => {
  const placement = placeGraphPopover(anchor(), { width: 900, height: 700 });
  assert.deepEqual(placement, {
    left: 296,
    width: 440,
    maxHeight: 280,
    top: 176,
    bottom: null,
    side: 'below',
  });
});

test('a short viewport opens the tag popover upward and clamps its size', () => {
  const placement = placeGraphPopover(anchor(), { width: 360, height: 270 });
  assert.equal(placement.side, 'above');
  assert.equal(placement.left, 12);
  assert.equal(placement.width, 336);
  assert.equal(placement.top, null);
  assert.equal(placement.bottom, 138);
  assert.ok(placement.maxHeight >= 24 && placement.maxHeight <= 246);
});

test('a right-edge anchor moves left instead of leaving the viewport', () => {
  const placement = placeGraphPopover(anchor({ left: 700, right: 760 }), {
    width: 760,
    height: 700,
  });
  assert.equal(placement.left, 308);
  assert.equal(placement.left + placement.width, 748);
});

test('an anchor outside the viewport suppresses the popover', () => {
  assert.equal(
    placeGraphPopover(anchor({ left: 500, right: 550 }), { width: 400, height: 700 }),
    null,
  );
  assert.equal(
    placeGraphPopover(anchor({ top: -60, bottom: -20 }), { width: 900, height: 700 }),
    null,
  );
});

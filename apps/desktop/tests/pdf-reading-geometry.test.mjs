import test from 'node:test';
import assert from 'node:assert/strict';
import {
  anchorToViewport,
  viewportToAnchor,
  pdfLayout,
  visiblePages,
  pageAtOffset,
  readingPosition,
  viewScrollPosition,
  PDF_MAX_RENDERED_PAGES,
} from '../src/sources/pdf-reading-geometry.ts';
import { ReadingQueue } from '../src/sources/reading-queue.ts';

const geometry = { crop: [20, 30, 620, 830], rotation: 0, userUnit: 1 };
const close = (a, b) => assert.ok(Math.abs(a - b) < 1e-5, `${a} != ${b}`);

test('unrotated document anchors round-trip for all intrinsic and view rotations', () => {
  for (const intrinsic of [0, 90, 180, 270])
    for (const extra of [0, 90, 180, 270]) {
      const point = anchorToViewport(0.37, 0.64, intrinsic + extra);
      const restored = viewportToAnchor(...point, intrinsic + extra);
      close(restored[0], 0.37);
      close(restored[1], 0.64);
    }
});

test('a hundred pages retain independent geometry while canvas windows stay bounded', () => {
  const metrics = new Map(
    Array.from({ length: 100 }, (_, index) => [index, { ...geometry, rotation: (index % 4) * 90 }]),
  );
  const layout = pdfLayout(100, metrics, geometry, 1.25, 90, 900);
  assert.equal(layout.pages.length, 100);
  assert.equal(layout.pages[0].width, 1000);
  assert.equal(layout.pages[1].height, 1000);
  for (const page of layout.pages) {
    assert.equal(pageAtOffset(layout.pages, page.top + 10), page.index);
    const visible = visiblePages(layout, page.top, 800);
    assert.ok(visible.some((item) => item.index === page.index));
    assert.ok(visible.length <= PDF_MAX_RENDERED_PAGES);
  }
});

test('PDF reading position records whether the scale is fit width, fit page or custom', () => {
  const layout = pdfLayout(1, new Map([[0, geometry]]), geometry, 1, 0, 700);
  for (const mode of ['width', 'page', 'custom']) {
    const position = readingPosition(layout, 'v', 1, 0, 0, 0, 700, null, mode);
    assert.equal(position.view.zoom_mode, mode);
  }
  assert.equal(readingPosition(layout, 'v', 1, 0, 0, 0, 700, null).view.zoom_mode, 'width');
});
test('page, within-page point, horizontal scroll and rotation restore after geometry updates', () => {
  const unknown = pdfLayout(
    100,
    new Map([
      [0, geometry],
      [80, geometry],
    ]),
    geometry,
    2,
    0,
    700,
  );
  const position = readingPosition(
    unknown,
    'version',
    2,
    0,
    unknown.pages[80].top + 333,
    300,
    700,
    null,
  );
  assert.equal(position.view.page_index, 80);
  assert.equal(position.locator.rects.length, 0);
  const known = new Map(
    Array.from({ length: 100 }, (_, index) => [
      index,
      index < 80 ? { ...geometry, rotation: 90 } : geometry,
    ]),
  );
  const current = pdfLayout(100, known, geometry, 2, 0, 700);
  const restored = viewScrollPosition(current, position.view, 700);
  const readback = readingPosition(
    current,
    'version',
    2,
    0,
    restored.top,
    restored.left,
    700,
    null,
  );
  assert.deepEqual(readback.view, position.view);
  assert.notEqual(restored.top, unknown.pages[80].top + 333);
});

test('close barrier captures a final PDF viewport before a pending scroll event', async () => {
  const metrics = new Map([[0, geometry]]),
    layout = pdfLayout(1, metrics, geometry, 2, 0, 700);
  let snapshot = readingPosition(layout, 'v', 2, 0, 0, 0, 700, null);
  let state = {
    source_version_id: 'v',
    status: 'reading',
    locator: null,
    pdf_view: null,
    row_version: 0,
  };
  const writes = [];
  const queue = new ReadingQueue(
    async () => state,
    async (current, locator, view, locatorReady) => {
      writes.push({ locator, view, locatorReady });
      state = {
        ...current,
        locator: locatorReady ? locator : null,
        pdf_view: view,
        row_version: current.row_version + 1,
      };
    },
  );
  queue.register('active', 'v', () => ({ ...snapshot, locatorReady: false }));
  queue.record('active', snapshot.locator, snapshot.view, false);
  snapshot = readingPosition(layout, 'v', 2, 0, 420, 170, 700, null);
  await queue.flush();
  assert.equal(writes.length, 1);
  assert.deepEqual(writes[0].view, snapshot.view);
  assert.equal(writes[0].locatorReady, false);
  assert.equal(state.locator, null);
  queue.release('active');
  await queue.flush();
  assert.equal(writes.length, 1);
});

test('unchanged page still saves final zoom and failed viewport remains retryable', async () => {
  const layout = pdfLayout(1, new Map([[0, geometry]]), geometry, 1, 0, 700);
  const first = readingPosition(layout, 'v', 1, 0, 0, 0, 700, null);
  const state = {
    source_version_id: 'v',
    status: 'reading',
    locator: first.locator,
    pdf_view: first.view,
    row_version: 7,
  };
  const next = { ...first.view, scale: 1.75, rotation: 90 };
  let fail = true;
  const writes = [];
  const queue = new ReadingQueue(
    async () => state,
    async (_, locator, view) => {
      if (fail) throw new Error('storage failed');
      writes.push({ locator, view });
    },
  );
  queue.activate('foreground', 'v');
  queue.record('foreground', first.locator, next);
  await assert.rejects(queue.flush(), /storage failed/);
  assert.equal(queue.hasPending(), true);
  fail = false;
  await queue.flush();
  assert.deepEqual(writes[0].view, next);
  assert.equal(queue.hasPending(), false);
});

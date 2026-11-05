import test from 'node:test';
import assert from 'node:assert/strict';
import { fitColumns, EDITOR_MIN, paneChrome } from '../src/layout/columns.ts';

test('resizing the window keeps room for writing and preserves preferred sizes', () => {
  const preferred = { sidebar_width: 360, note_list_width: 540 };
  for (const width of [720, 780, 960, 1280, 1440, 1920]) {
    const result = fitColumns(width, true, true, preferred);
    assert.ok(result.sidebar >= 160);
    assert.ok(result.list >= 200);
    assert.ok(width - result.sidebar - result.list - 1 - paneChrome(true) >= EDITOR_MIN);
    assert.ok(result.sidebar <= result.sidebarMax);
    assert.ok(result.list <= result.listMax);
  }
  assert.deepEqual(preferred, { sidebar_width: 360, note_list_width: 540 });
  assert.equal(fitColumns(1920, true, true, preferred).list, 540);
});
test('zoomed windows undock navigation before it can squeeze the document away', () => {
  const preferred = { sidebar_width: 400, note_list_width: 600 };
  for (const width of [240, 360, 480, 489, 490, 538, 539, 690, 691, 720, 1080, 1920]) {
    for (const sidebar of [true, false]) {
      for (const list of [true, false]) {
        const result = fitColumns(width, sidebar, list, preferred);
        const documentWidth =
          width -
          result.sidebar -
          result.list -
          Number(result.list > 0) -
          paneChrome(result.sidebar > 0);
        assert.ok(documentWidth >= Math.min(EDITOR_MIN, width - paneChrome(false)));
        assert.ok(!result.sidebar || result.sidebar >= 160);
        assert.ok(!result.list || result.list >= 200);
      }
    }
  }
  const zoomed = fitColumns(360, true, true, preferred);
  assert.equal(zoomed.sidebar, 0);
  assert.equal(zoomed.list, 0);
  assert.equal(zoomed.sidebarAllowed, false);
  assert.equal(zoomed.listAllowed, false);
  assert.equal(fitColumns(690, true, true, preferred).list, 0);
  assert.equal(fitColumns(691, true, true, preferred).list, 200);
  assert.equal(fitColumns(539, false, true, preferred).list, 200);
  assert.deepEqual(preferred, { sidebar_width: 400, note_list_width: 600 });
  assert.equal(fitColumns(1920, true, true, preferred).list, 600);
});
test('hidden sidebar, gallery and focus mode release their column width', () => {
  const preferred = { sidebar_width: 300, note_list_width: 500 };
  assert.equal(fitColumns(1000, false, true, preferred).sidebar, 0);
  assert.equal(fitColumns(1000, true, false, preferred).list, 0);
  const focused = fitColumns(720, false, false, preferred);
  assert.equal(focused.sidebar + focused.list, 0);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { parseFragment } from 'parse5';
import { isolatedSsrCache } from './ssr-test-cache.mjs';
import {
  reviewSnoozeView,
  snoozeKeyAction,
  snoozeMinimum,
  toggleSnoozePanel,
  withSnoozeDate,
} from '../src/review/snooze-panel.ts';
import { snoozeTimestamp } from '../src/review/review-intents.ts';

/** React SSR separates adjacent text nodes with `<!-- -->`; that is not copy. */
const copy = (html) => html.replace(/<!--[\s\S]*?-->/g, '');

// The defect this locks: the page used to keep one free-standing `date` string and
// a separate `snoozing` flag, so a time typed for one entry stayed in the field
// after switching to another entry (and after paging) and could be submitted for
// the entry that happened to be selected by then.
test('a 稍后 date belongs to the entry it was typed for', () => {
  const typed = { entryId: 'entry-a', date: '2026-11-01T09:00' };
  assert.deepEqual(reviewSnoozeView(typed, 'entry-a'), {
    open: true,
    date: '2026-11-01T09:00',
  });
  assert.deepEqual(reviewSnoozeView(typed, 'entry-b'), { open: false, date: '' });
  assert.deepEqual(reviewSnoozeView(typed, null), { open: false, date: '' });
  assert.deepEqual(reviewSnoozeView(null, 'entry-a'), { open: false, date: '' });
});

test('the panel starts empty for every other entry, and closing drops the date', () => {
  // Opening for another entry never carries the previous one's date over.
  assert.deepEqual(toggleSnoozePanel({ entryId: 'entry-a', date: '2026-11-01T09:00' }, 'entry-b'), {
    entryId: 'entry-b',
    date: '',
  });
  // The toggle closes the panel it belongs to, and reopening starts empty.
  assert.equal(
    toggleSnoozePanel({ entryId: 'entry-a', date: '2026-11-01T09:00' }, 'entry-a'),
    null,
  );
  assert.deepEqual(toggleSnoozePanel(null, 'entry-a'), { entryId: 'entry-a', date: '' });

  const typed = withSnoozeDate(null, 'entry-a', '2026-11-01T09:00');
  assert.deepEqual(reviewSnoozeView(typed, 'entry-a'), {
    open: true,
    date: '2026-11-01T09:00',
  });
  assert.deepEqual(
    reviewSnoozeView(withSnoozeDate(typed, 'entry-b', '2026-12-24T18:30'), 'entry-b'),
    {
      open: true,
      date: '2026-12-24T18:30',
    },
  );
  assert.deepEqual(
    reviewSnoozeView(withSnoozeDate(typed, 'entry-b', '2026-12-24T18:30'), 'entry-a'),
    {
      open: false,
      date: '',
    },
  );
});

test('typing, closing and reopening leaves an empty field', () => {
  let panel = toggleSnoozePanel(null, 'entry-a'); // open
  panel = withSnoozeDate(panel, 'entry-a', '2026-11-01T09:00'); // type
  assert.equal(reviewSnoozeView(panel, 'entry-a').date, '2026-11-01T09:00');
  panel = toggleSnoozePanel(panel, 'entry-a'); // close
  assert.equal(panel, null);
  panel = toggleSnoozePanel(panel, 'entry-a'); // reopen
  assert.deepEqual(
    reviewSnoozeView(panel, 'entry-a'),
    { open: true, date: '' },
    'the reopened field is empty',
  );
  assert.equal(panel.date, '');
});

// A time equal to the current minute is not in the future, so `snoozeTimestamp`
// refuses it: the field's minimum must therefore be the next minute, not this one.
test('the field minimum is the next local minute and always submits', () => {
  assert.equal(snoozeMinimum(new Date(2026, 9, 16, 10, 30, 45)), '2026-10-16T10:31');
  assert.equal(snoozeMinimum(new Date(2026, 9, 16, 10, 30, 0)), '2026-10-16T10:31');
  assert.equal(snoozeMinimum(new Date(2026, 0, 5, 8, 5, 3)), '2026-01-05T08:06');
  assert.equal(snoozeMinimum(new Date(2026, 0, 31, 23, 59, 59)), '2026-02-01T00:00');
  for (const now of [
    new Date(2026, 9, 16, 10, 30, 0),
    new Date(2026, 9, 16, 10, 30, 45),
    new Date(2026, 11, 31, 23, 59, 59),
  ]) {
    const minimum = snoozeMinimum(now);
    assert.ok(
      new Date(snoozeTimestamp(minimum, now.getTime())).getTime() > now.getTime(),
      `${minimum} must be a future time the submit accepts`,
    );
  }
  // This is the trap the minimum avoids: the minute the user is living in cannot
  // be submitted, so offering it as a valid choice would only produce an error.
  assert.throws(
    () => snoozeTimestamp('2026-10-16T10:30', new Date(2026, 9, 16, 10, 30, 45).getTime()),
    /未来的有效时间/,
  );
});

test('only Escape leaves the panel: Enter and Tab stay the field and the form', () => {
  assert.equal(snoozeKeyAction('Escape'), 'cancel');
  assert.equal(snoozeKeyAction('Enter'), null);
  assert.equal(snoozeKeyAction('Tab'), null);
  assert.equal(snoozeKeyAction('esc'), null);
});

test('the page keeps the panel in the shared state and closes it on a new selection', () => {
  const source = readFileSync(new URL('../src/review/ReviewPage.tsx', import.meta.url), 'utf8');
  assert.ok(/<SnoozeForm/.test(source), 'the page renders the extracted field');
  assert.ok(/toggleSnoozePanel\(/.test(source), 'the 稍后 button goes through the shared toggle');
  assert.ok(/withSnoozeDate\(/.test(source), 'typing stays bound to the entry');
  assert.ok(/snoozeMinimum\(/.test(source), 'the field carries a minimum');
  assert.ok(
    /setSnooze\(null\)[\s\S]{0,60}\[selected, setSnooze\]/.test(source),
    'a new selection closes it',
  );
  assert.ok(
    !/const \[date, setDate\]/.test(source),
    'the page no longer owns a date that outlives the panel',
  );
  assert.ok(/aria-expanded=\{panel\.open\}/.test(source), 'the button announces the panel');
});

test('the field renders a future-only, focused input with its label and submit', async (context) => {
  const server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('..', import.meta.url)),
    cacheDir: await isolatedSsrCache(context),
    logLevel: 'error',
    esbuild: { jsx: 'automatic' },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    appType: 'custom',
  });
  try {
    const form = await server.ssrLoadModule('/src/review/SnoozeForm.tsx');
    const i18n = await server.ssrLoadModule('/src/i18n.tsx');
    const render = (props, language = 'zh-CN') =>
      copy(
        renderToStaticMarkup(
          createElement(
            i18n.LanguageContext.Provider,
            { value: language },
            createElement(form.SnoozeForm, {
              min: '2026-10-16T10:31',
              disabled: false,
              onDate() {},
              onSubmit() {},
              onCancel() {},
              ...props,
            }),
          ),
        ),
      );
    const parts = (html) => {
      const node = elements(parseFragment(html)).find((element) =>
        classes(element).includes('review-snooze'),
      );
      assert.ok(node, 'the panel renders one form');
      const [label, submit] = elements(node);
      const input = elements(label).find((element) => element.tagName === 'input');
      assert.ok(input, 'the label wraps the field');
      return { label, input, submit };
    };

    const empty = parts(render({ date: '' }));
    assert.equal(attribute(empty.input, 'type'), 'datetime-local');
    assert.equal(attribute(empty.input, 'min'), '2026-10-16T10:31');
    assert.equal(attribute(empty.input, 'value'), '');
    assert.ok(has(empty.input, 'required'), 'a time is required');
    assert.ok(has(empty.input, 'autofocus'), 'opening the panel puts the caret in the field');
    assert.equal(text(empty.label).trim(), '下次回顾时间');
    assert.equal(text(empty.submit), '确认时间');
    assert.ok(has(empty.submit, 'disabled'), 'an empty field cannot submit');

    const typed = parts(render({ date: '2026-11-01T09:00' }));
    assert.equal(attribute(typed.input, 'value'), '2026-11-01T09:00');
    assert.ok(!has(typed.submit, 'disabled'), 'a chosen time submits');

    const locked = parts(render({ date: '2026-11-01T09:00', disabled: true }));
    assert.ok(has(locked.input, 'disabled'));
    assert.ok(has(locked.submit, 'disabled'));

    const english = parts(render({ date: '' }, 'en'));
    assert.equal(text(english.label).trim(), 'Next review');
    assert.equal(text(english.submit), 'Set time');
  } finally {
    await server.close();
  }
});

/** Element children only: text nodes and comments cannot carry layout. */
function elements(node) {
  return (node.childNodes ?? []).filter((child) => typeof child.tagName === 'string');
}
function classes(node) {
  const attribute = (node.attrs ?? []).find((entry) => entry.name === 'class');
  return (attribute?.value ?? '').split(/\s+/).filter(Boolean);
}
function attribute(node, name) {
  return (node.attrs ?? []).find((entry) => entry.name === name)?.value ?? null;
}
function has(node, name) {
  return (node.attrs ?? []).some((entry) => entry.name === name);
}
function text(node) {
  if (node.nodeName === '#text') return node.value ?? '';
  return (node.childNodes ?? []).map(text).join('');
}

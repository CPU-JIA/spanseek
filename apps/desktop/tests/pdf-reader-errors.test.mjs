import test from 'node:test';
import assert from 'node:assert/strict';
import {
  PdfOriginalReadError,
  pdfErrorExcerpt,
  pdfReaderErrorMessage,
} from '../src/sources/pdf-reader-errors.ts';

const zh = (chinese) => chinese;
const en = (_chinese, english) => english;

test('native ApiError codes distinguish read failure, unavailable authorization and failed integrity without leaking messages', () => {
  const privateMessage =
    'C:\\Users\\private-user\\notes\\private-file.pdf http://pkms-source.localhost/private-ticket';
  assert.match(
    pdfReaderErrorMessage({ code: 'io_failed', message: privateMessage }, zh),
    /读取失败/,
  );
  assert.match(
    pdfReaderErrorMessage({ code: 'io_failed', message: privateMessage }, en),
    /could not be read/,
  );
  assert.match(
    pdfReaderErrorMessage({ code: 'not_found', message: privateMessage }, en),
    /read authorization is unavailable/,
  );
  assert.match(
    pdfReaderErrorMessage({ code: 'integrity_failed', message: privateMessage }, zh),
    /完整性校验/,
  );
  assert.match(pdfReaderErrorMessage({ code: 'instance_mismatch' }, en), /workspace changed/);
  for (const failure of [
    { code: 'io_failed', message: privateMessage },
    { code: 'not_found', message: privateMessage },
    { code: 'internal', message: privateMessage },
    { code: 'future_code', message: privateMessage },
    new Error(privateMessage),
    privateMessage,
  ]) {
    for (const translate of [zh, en]) {
      const text = pdfReaderErrorMessage(failure, translate);
      assert.doesNotMatch(text, /private-user|private-file|private-ticket|pkms-source|C:\\Users/);
      assert.doesNotMatch(text, /missing|deleted|已丢失|已删除/);
    }
  }
});

test('range and parser failures retain distinct safe retry guidance', () => {
  assert.match(
    pdfReaderErrorMessage(new PdfOriginalReadError('pdf_resource_unavailable'), en),
    /authorization is unavailable/,
  );
  assert.match(
    pdfReaderErrorMessage(new PdfOriginalReadError('pdf_range_mismatch'), zh),
    /范围响应不匹配/,
  );
  assert.match(
    pdfReaderErrorMessage(new PdfOriginalReadError('pdf_range_incomplete'), en),
    /range was incomplete/,
  );
  assert.match(
    pdfReaderErrorMessage({ name: 'PasswordException', message: 'secret' }, en),
    /requires a password/,
  );
  assert.match(
    pdfReaderErrorMessage({ name: 'InvalidPDFException', message: 'secret' }, zh),
    /无法解析/,
  );
});

test('error snapshot preserves the exact quote and page only for the selected fixed PDF version', () => {
  const locator = {
    kind: 'pdf',
    source_version_id: 'original-version',
    page_index: 3,
    page_label: 'iv',
    quote: '原版第一行\nSecond line 😀',
  };
  assert.deepEqual(pdfErrorExcerpt('original-version', locator), {
    quote: locator.quote,
    page: 'iv',
  });
  assert.deepEqual(pdfErrorExcerpt('original-version', { ...locator, page_label: null }), {
    quote: locator.quote,
    page: '4',
  });
  assert.equal(pdfErrorExcerpt('new-version', locator), null);
  assert.equal(pdfErrorExcerpt('original-version', { ...locator, kind: 'text' }), null);
  assert.equal(pdfErrorExcerpt('original-version', null), null);
});

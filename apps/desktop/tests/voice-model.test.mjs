import assert from 'node:assert/strict';
import test from 'node:test';
import {
  formatDuration,
  levelFromWaveform,
  suggestCaptureTitle,
  toSrt,
  toVtt,
  transcriptCues,
  transcriptPreview,
  waveformPeaks,
} from '../src/voice/voice-model.ts';
import { pickMimeType } from '../src/voice/recorder.ts';

test('duration format stays compact and tabular', () => {
  assert.equal(formatDuration(0), '0:00');
  assert.equal(formatDuration(9_400), '0:09');
  assert.equal(formatDuration(65_000), '1:05');
  assert.equal(formatDuration(3_723_000), '1:02:03');
  assert.equal(formatDuration(Number.NaN), '0:00');
});

test('waveform level is bounded and scales quiet input', () => {
  assert.equal(levelFromWaveform(new Uint8Array()), 0);
  assert.equal(levelFromWaveform(new Uint8Array([128, 128, 128, 128])), 0);
  const loud = levelFromWaveform(new Uint8Array([255, 0, 255, 0]));
  assert.ok(loud > 0.9, `expected loud level, got ${loud}`);
  assert.ok(levelFromWaveform(new Uint8Array([132, 124, 130, 126])) < 0.2);
});

test('preview collapses whitespace, bounds length and marks truncation', () => {
  assert.equal(transcriptPreview('  第一行\n\n第二行  '), '第一行 第二行');
  const long = '灵感'.repeat(100);
  const preview = transcriptPreview(long, 20);
  assert.equal([...preview].length, 20);
  assert.ok(preview.endsWith('…'));
});

test('a title is suggested only from actual transcript text', () => {
  assert.equal(suggestCaptureTitle(' \n 第一点：先做录音\n第二点'), '第一点：先做录音');
  assert.equal(suggestCaptureTitle('   \n'), '');
  assert.equal([...suggestCaptureTitle('长'.repeat(80))].length, 40);
});

test('recorder picks the first supported container', () => {
  assert.equal(
    pickMimeType((type) => type === 'audio/webm;codecs=opus' || type === 'audio/webm'),
    'audio/webm;codecs=opus',
  );
  assert.equal(pickMimeType((type) => type === 'audio/mp4'), 'audio/mp4');
  assert.equal(pickMimeType(() => false), null);
});

test('waveform buckets preserve peak amplitude', () => {
  const samples = new Float32Array([0, 0.1, -0.8, 0.2, 0.05, 0, -1, 0.3]);
  const peaks = waveformPeaks(samples, 4);
  for (const [index, expected] of [0.1, 0.8, 0.05, 1].entries()) {
    assert.ok(Math.abs(peaks[index] - expected) < 1e-6);
  }
  assert.equal(waveformPeaks(new Float32Array(), 3).length, 3);
});

test('subtitle cues follow line order and stay within duration', () => {
  const cues = transcriptCues('第一行\n第二行\n第三行', 9_000);
  assert.equal(cues.length, 3);
  assert.equal(cues[0].startMs, 0);
  assert.equal(cues[2].endMs, 9_000);
  assert.ok(cues[1].startMs >= cues[0].endMs - 1);
  assert.equal(transcriptCues('   ', 1_000).length, 0);
});

test('SRT and VTT keep standard headers and timestamps', () => {
  const cues = transcriptCues('第一行\n第二行', 5_000);
  const srt = toSrt(cues);
  assert.match(srt, /^1\n00:00:00,000 --> /);
  assert.match(srt, /2\n/);
  const vtt = toVtt(cues);
  assert.ok(vtt.startsWith('WEBVTT\n\n'));
  assert.match(vtt, /00:00:00\.000 --> /);
  assert.ok(!vtt.includes(','));
});

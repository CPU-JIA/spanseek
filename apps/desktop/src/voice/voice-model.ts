// Pure helpers shared by the voice page and its tests.
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '0:00';
  const total = Math.floor(ms / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  if (hours > 0) return `${hours}:${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
  return `${minutes}:${String(seconds).padStart(2, '0')}`;
}

/** Root-mean-square of a time-domain buffer, scaled to a usable 0..1 meter. */
export function levelFromWaveform(samples: Uint8Array): number {
  if (samples.length === 0) return 0;
  let sum = 0;
  for (const sample of samples) {
    const centered = (sample - 128) / 128;
    sum += centered * centered;
  }
  const rms = Math.sqrt(sum / samples.length);
  return Math.min(1, Math.max(0, rms * 3.2));
}

export function transcriptPreview(transcript: string, maximum = 90): string {
  const text = transcript.replace(/\s+/g, ' ').trim();
  if (text.length <= maximum) return text;
  return `${text.slice(0, Math.max(0, maximum - 1)).trimEnd()}…`;
}

export function suggestCaptureTitle(transcript: string): string {
  const line = transcript
    .split(/\r?\n/)
    .map((value) => value.trim())
    .find(Boolean);
  if (!line) return '';
  return line.slice(0, 40);
}

export function base64FromBlob(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('voice_audio_read_failed'));
    reader.onload = () => {
      const value = typeof reader.result === 'string' ? reader.result : '';
      const comma = value.indexOf(',');
      if (comma < 0) {
        reject(new Error('voice_audio_read_failed'));
        return;
      }
      resolve(value.slice(comma + 1));
    };
    reader.readAsDataURL(blob);
  });
}

export function transcriptFromCapture(capture: { transcript: string }): string {
  return capture.transcript;
}

/** Bucket peak amplitudes for a compact waveform; input may be any channel. */
export function waveformPeaks(samples: Float32Array, buckets = 64): number[] {
  if (buckets <= 0) return [];
  if (samples.length === 0) return new Array(buckets).fill(0);
  const peaks: number[] = [];
  const width = Math.max(1, Math.floor(samples.length / buckets));
  for (let bucket = 0; bucket < buckets; bucket += 1) {
    const start = bucket * width;
    const end = bucket === buckets - 1 ? samples.length : Math.min(samples.length, start + width);
    let peak = 0;
    for (let index = start; index < end; index += 1) {
      const value = Math.abs(samples[index]);
      if (value > peak) peak = value;
    }
    peaks.push(Math.min(1, peak));
  }
  return peaks;
}

/** Decode only the first two minutes so a four-hour import cannot freeze the page. */
export async function decodeWaveform(blob: Blob, buckets = 64): Promise<number[]> {
  const context = new AudioContext();
  try {
    const bytes = await blob.arrayBuffer();
    const buffer = await context.decodeAudioData(bytes.slice(0));
    const channel = buffer.getChannelData(0);
    const sampleLimit = Math.min(channel.length, buffer.sampleRate * 120);
    return waveformPeaks(channel.subarray(0, sampleLimit), buckets);
  } finally {
    void context.close();
  }
}

export type TranscriptCue = { startMs: number; endMs: number; text: string };

/** Derive approximate cues from lines when the provider returned plain text only. */
export function transcriptCues(transcript: string, durationMs: number): TranscriptCue[] {
  const lines = transcript
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length === 0) return [];
  const totalWeight = lines.reduce((sum, line) => sum + Math.max(1, line.length), 0);
  const total = durationMs > 0 ? durationMs : lines.length * 4_000;
  let cursor = 0;
  return lines.map((line, index) => {
    const share = index === lines.length - 1 ? total - cursor : (total * Math.max(1, line.length)) / totalWeight;
    const startMs = cursor;
    cursor = Math.min(total, Math.round(cursor + Math.max(600, share)));
    return { startMs, endMs: cursor, text: line };
  });
}

function subtitleTimestamp(ms: number, separator: ',' | '.'): string {
  const total = Math.max(0, Math.round(ms));
  const hours = Math.floor(total / 3_600_000);
  const minutes = Math.floor((total % 3_600_000) / 60_000);
  const seconds = Math.floor((total % 60_000) / 1_000);
  const millis = total % 1_000;
  return `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}${separator}${String(millis).padStart(3, '0')}`;
}

export function toSrt(cues: readonly TranscriptCue[]): string {
  return cues
    .map(
      (cue, index) =>
        `${index + 1}\n${subtitleTimestamp(cue.startMs, ',')} --> ${subtitleTimestamp(cue.endMs, ',')}\n${cue.text}`,
    )
    .join('\n\n');
}

export function toVtt(cues: readonly TranscriptCue[]): string {
  return `WEBVTT\n\n${cues
    .map(
      (cue) =>
        `${subtitleTimestamp(cue.startMs, '.')} --> ${subtitleTimestamp(cue.endMs, '.')}\n${cue.text}`,
    )
    .join('\n\n')}`;
}

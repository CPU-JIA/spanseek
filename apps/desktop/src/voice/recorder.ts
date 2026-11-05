// Microphone capture is kept in one place so permission and hardware failures
// are mapped to actionable codes instead of browser-specific messages.
export type RecorderErrorCode =
  | 'microphone_denied'
  | 'microphone_missing'
  | 'microphone_busy'
  | 'recorder_unsupported'
  | 'recording_failed'
  | 'system_audio_denied'
  | 'system_audio_unavailable';

export class RecorderError extends Error {
  readonly code: RecorderErrorCode;

  constructor(code: RecorderErrorCode) {
    super(code);
    this.name = 'RecorderError';
    this.code = code;
  }
}

export type AudioInput = { deviceId: string; label: string };

export type RecordingResult = {
  blob: Blob;
  durationMs: number;
  mimeType: string;
};

export type ActiveRecording = {
  stream: MediaStream;
  mimeType: string;
  paused: boolean;
  elapsedMs: () => number;
  pause: () => void;
  resume: () => void;
  stop: () => Promise<RecordingResult>;
  cancel: () => void;
};

const CANDIDATE_TYPES = [
  'audio/webm;codecs=opus',
  'audio/webm',
  'audio/ogg;codecs=opus',
  'audio/mp4',
] as const;

export function pickMimeType(supported: (type: string) => boolean): string | null {
  return CANDIDATE_TYPES.find((type) => supported(type)) ?? null;
}

export async function listAudioInputs(): Promise<AudioInput[]> {
  if (!navigator.mediaDevices?.enumerateDevices) return [];
  const devices = await navigator.mediaDevices.enumerateDevices();
  return devices
    .filter((device) => device.kind === 'audioinput')
    .map((device, index) => ({
      deviceId: device.deviceId,
      label: device.label || `Microphone ${index + 1}`,
    }));
}

export async function startRecording(options?: {
  deviceId?: string | null;
  onChunk?: (chunk: Blob) => void;
}): Promise<ActiveRecording> {
  if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === 'undefined') {
    throw new RecorderError('recorder_unsupported');
  }
  const mimeType = pickMimeType((type) => MediaRecorder.isTypeSupported(type));
  if (!mimeType) throw new RecorderError('recorder_unsupported');

  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        ...(options?.deviceId ? { deviceId: { exact: options.deviceId } } : {}),
        channelCount: 1,
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      },
    });
  } catch (error) {
    throw mapGetUserMediaError(error);
  }

  let recorder: MediaRecorder;
  try {
    recorder = new MediaRecorder(stream, { mimeType, audioBitsPerSecond: 48_000 });
  } catch {
    release(stream);
    throw new RecorderError('recorder_unsupported');
  }
  const chunks: Blob[] = [];
  const startedAt = performance.now();
  let pausedAt: number | null = null;
  let pausedTotal = 0;
  let settled = false;
  recorder.addEventListener('dataavailable', (event) => {
    if (event.data.size === 0) return;
    chunks.push(event.data);
    options?.onChunk?.(event.data);
  });
  try {
    recorder.start(1_000);
  } catch {
    release(stream);
    throw new RecorderError('recording_failed');
  }

  const elapsedMs = () => {
    const pausedNow = pausedAt === null ? 0 : performance.now() - pausedAt;
    return Math.max(0, Math.round(performance.now() - startedAt - pausedTotal - pausedNow));
  };
  const finish = () => {
    if (settled) return;
    settled = true;
    release(stream);
    const type = recorder.mimeType || mimeType;
    return {
      blob: new Blob(chunks, { type }),
      durationMs: elapsedMs(),
      mimeType: type,
    } satisfies RecordingResult;
  };
  const stop = () =>
    new Promise<RecordingResult>((resolve) => {
      if (settled) {
        resolve({
          blob: new Blob([], { type: mimeType }),
          durationMs: elapsedMs(),
          mimeType,
        });
        return;
      }
      const complete = () => resolve(finish()!);
      recorder.addEventListener('stop', complete, { once: true });
      recorder.addEventListener('error', complete, { once: true });
      if (recorder.state === 'inactive') {
        complete();
      } else {
        try {
          recorder.stop();
        } catch {
          complete();
        }
      }
    });

  return {
    stream,
    mimeType,
    get paused() {
      return pausedAt !== null;
    },
    elapsedMs,
    pause: () => {
      if (settled || pausedAt !== null || recorder.state !== 'recording') return;
      try {
        recorder.pause();
        pausedAt = performance.now();
      } catch {
        // The next stop still finalizes whatever was captured.
      }
    },
    resume: () => {
      if (settled || pausedAt === null || recorder.state !== 'paused') return;
      try {
        recorder.resume();
        pausedTotal += performance.now() - pausedAt;
        pausedAt = null;
      } catch {
        pausedAt = null;
      }
    },
    stop,
    cancel: () => {
      if (settled) return;
      settled = true;
      chunks.length = 0;
      try {
        if (recorder.state !== 'inactive') recorder.stop();
      } catch {
        // The stream is released below regardless.
      }
      release(stream);
    },
  };
}

function release(stream: MediaStream) {
  stream.getTracks().forEach((track) => track.stop());
}

function mapGetUserMediaError(error: unknown): RecorderError {
  const name =
    error && typeof error === 'object' && 'name' in error
      ? String((error as { name?: unknown }).name)
      : '';
  if (name === 'NotAllowedError' || name === 'SecurityError') {
    return new RecorderError('microphone_denied');
  }
  if (name === 'NotFoundError' || name === 'OverconstrainedError') {
    return new RecorderError('microphone_missing');
  }
  if (name === 'NotReadableError' || name === 'AbortError') {
    return new RecorderError('microphone_busy');
  }
  return new RecorderError('recording_failed');
}

/** Mix microphone and system audio into one recording for meetings. */
export async function startMeetingRecording(options?: {
  deviceId?: string | null;
}): Promise<ActiveRecording> {
  if (
    !navigator.mediaDevices?.getUserMedia ||
    !navigator.mediaDevices.getDisplayMedia ||
    typeof MediaRecorder === 'undefined'
  ) {
    throw new RecorderError('system_audio_unavailable');
  }
  const mimeType = pickMimeType((type) => MediaRecorder.isTypeSupported(type));
  if (!mimeType) throw new RecorderError('recorder_unsupported');
  let mic: MediaStream;
  try {
    mic = await navigator.mediaDevices.getUserMedia({
      audio: {
        ...(options?.deviceId ? { deviceId: { exact: options.deviceId } } : {}),
        channelCount: 1,
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      },
    });
  } catch (error) {
    throw mapGetUserMediaError(error);
  }
  let display: MediaStream;
  try {
    display = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: true });
  } catch (error) {
    release(mic);
    throw mapDisplayMediaError(error);
  }
  const systemTracks = display.getAudioTracks();
  if (systemTracks.length === 0) {
    release(mic);
    release(display);
    throw new RecorderError('system_audio_unavailable');
  }
  display.getVideoTracks().forEach((track) => track.stop());
  const context = new AudioContext();
  const destination = context.createMediaStreamDestination();
  context.createMediaStreamSource(mic).connect(destination);
  context.createMediaStreamSource(new MediaStream(systemTracks)).connect(destination);
  let recorder: MediaRecorder;
  try {
    recorder = new MediaRecorder(destination.stream, {
      mimeType,
      audioBitsPerSecond: 64_000,
    });
  } catch {
    release(mic);
    release(display);
    void context.close();
    throw new RecorderError('recorder_unsupported');
  }
  const chunks: Blob[] = [];
  const startedAt = performance.now();
  let pausedAt: number | null = null;
  let pausedTotal = 0;
  let settled = false;
  recorder.addEventListener('dataavailable', (event) => {
    if (event.data.size > 0) chunks.push(event.data);
  });
  try {
    recorder.start(1_000);
  } catch {
    release(mic);
    release(display);
    void context.close();
    throw new RecorderError('recording_failed');
  }
  const cleanup = () => {
    release(mic);
    release(display);
    destination.stream.getTracks().forEach((track) => track.stop());
    void context.close();
  };
  const elapsedMs = () => {
    const pausedNow = pausedAt === null ? 0 : performance.now() - pausedAt;
    return Math.max(0, Math.round(performance.now() - startedAt - pausedTotal - pausedNow));
  };
  const finish = () => {
    if (settled) return;
    settled = true;
    cleanup();
    return {
      blob: new Blob(chunks, { type: recorder.mimeType || mimeType }),
      durationMs: elapsedMs(),
      mimeType: recorder.mimeType || mimeType,
    } satisfies RecordingResult;
  };
  const stop = () =>
    new Promise<RecordingResult>((resolve) => {
      if (settled) {
        resolve({ blob: new Blob([], { type: mimeType }), durationMs: elapsedMs(), mimeType });
        return;
      }
      const complete = () => resolve(finish()!);
      recorder.addEventListener('stop', complete, { once: true });
      recorder.addEventListener('error', complete, { once: true });
      if (recorder.state === 'inactive') complete();
      else {
        try {
          recorder.stop();
        } catch {
          complete();
        }
      }
    });

  return {
    stream: destination.stream,
    mimeType,
    get paused() {
      return pausedAt !== null;
    },
    elapsedMs,
    pause: () => {
      if (settled || pausedAt !== null || recorder.state !== 'recording') return;
      try {
        recorder.pause();
        pausedAt = performance.now();
      } catch {
        // Stop still finalizes whatever was captured.
      }
    },
    resume: () => {
      if (settled || pausedAt === null || recorder.state !== 'paused') return;
      try {
        recorder.resume();
        pausedTotal += performance.now() - pausedAt;
        pausedAt = null;
      } catch {
        pausedAt = null;
      }
    },
    stop,
    cancel: () => {
      if (settled) return;
      settled = true;
      chunks.length = 0;
      try {
        if (recorder.state !== 'inactive') recorder.stop();
      } catch {
        // Cleanup below still releases every track.
      }
      cleanup();
    },
  };
}

function mapDisplayMediaError(error: unknown): RecorderError {
  const name =
    error && typeof error === 'object' && 'name' in error
      ? String((error as { name?: unknown }).name)
      : '';
  if (name === 'NotAllowedError' || name === 'SecurityError') {
    return new RecorderError('system_audio_denied');
  }
  return new RecorderError('system_audio_unavailable');
}

/** Read only duration metadata; a missing duration is not a reason to reject audio. */
export function readAudioDuration(file: Blob): Promise<number> {
  return new Promise((resolve) => {
    const url = URL.createObjectURL(file);
    const audio = document.createElement('audio');
    const finish = (duration: number) => {
      URL.revokeObjectURL(url);
      audio.removeAttribute('src');
      resolve(Number.isFinite(duration) && duration > 0 ? Math.round(duration * 1000) : 0);
    };
    audio.preload = 'metadata';
    audio.addEventListener('loadedmetadata', () => {
      if (Number.isFinite(audio.duration)) {
        finish(audio.duration);
        return;
      }
      // WebM without a Cues duration often reports Infinity; seek past the end
      // once so the browser can resolve the real value.
      audio.currentTime = Number.MAX_SAFE_INTEGER;
    });
    audio.addEventListener('durationchange', () => {
      if (Number.isFinite(audio.duration)) finish(audio.duration);
    });
    audio.addEventListener('error', () => finish(0));
    audio.src = url;
  });
}

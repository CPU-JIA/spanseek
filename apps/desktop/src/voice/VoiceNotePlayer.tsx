import { useEffect, useRef, useState } from 'react';
import type { VoiceCapture, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { formatDuration } from './voice-model';
import { loadVoiceAudio, saveVoiceAudio } from './api';
import { AudioPlayer } from '../ui/AudioPlayer';
import './voice-note-player.css';

/**
 * A promoted voice note reuses its capture UUID, so the editor can resolve the
 * original audio without parsing YAML. Non-voice notes simply render nothing.
 */
export function VoiceNotePlayer({
  identity,
  noteId,
}: {
  identity: WorkspaceIdentity;
  noteId: string;
}) {
  const { t } = useI18n();
  const [capture, setCapture] = useState<VoiceCapture | null>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState('');
  const urlRef = useRef<string | null>(null);
  const identityKey = `${identity.local_instance_id}/${identity.workspace_id}`;

  useEffect(() => {
    let cancelled = false;
    setCapture(null);
    setUrl(null);
    setError('');
    void call(
      { kind: 'voice_capture_get', payload: { capture_id: noteId } },
      'voice_capture',
      identity,
    )
      .then(async (value) => {
        if (cancelled || value.note_id !== noteId) return;
        setCapture(value);
        const bytes = await loadVoiceAudio(identity, noteId);
        if (cancelled) return;
        const next = URL.createObjectURL(new Blob([bytes], { type: value.audio_media_type }));
        urlRef.current = next;
        setUrl(next);
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        // A normal note has no voice capture; only a real capture failure is shown.
        const code =
          (reason as { code?: string })?.code ??
          (reason as { details?: { code?: string } })?.details?.code;
        if (code !== 'not_found') {
          setError(
            t(
              '无法读取原始语音，请打开语音速记重试。',
              'Could not load the original audio. Open Voice capture and retry.',
            ),
          );
        }
      });
    return () => {
      cancelled = true;
      if (urlRef.current) {
        URL.revokeObjectURL(urlRef.current);
        urlRef.current = null;
      }
    };
    // Identity is an object recreated by the shell; the stable IDs define scope.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [identityKey, noteId]);

  if (!capture) return null;
  return (
    <section className="voice-note-player" aria-label={t('语音原件', 'Original voice audio')}>
      <header>
        <span>{t('语音原件', 'Original voice audio')}</span>
        <span>{formatDuration(capture.duration_ms)}</span>
        {capture.status === 'failed' && (
          <span className="voice-note-player-warning">{t('待转写', 'Needs transcript')}</span>
        )}
      </header>
      {url ? (
        <AudioPlayer
          src={url}
          durationMs={capture.duration_ms}
          label={t('语音原件回放', 'Voice audio playback')}
          onSave={() => saveVoiceAudio(identity, noteId)}
        />
      ) : (
        <p role="status">{t('正在读取语音…', 'Loading audio…')}</p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

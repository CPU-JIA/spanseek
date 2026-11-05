import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { Download, MoreHorizontal, Pause, Play, Volume2, VolumeX } from 'lucide-react';
import { useI18n } from '../i18n';
import { formatDuration } from '../voice/voice-model';
import { PopupMenu } from './PopupMenu';
import './audio-player.css';

export const AudioPlayer = forwardRef<
  HTMLAudioElement,
  {
    src: string;
    label: string;
    durationMs: number;
    onSave: () => Promise<boolean>;
  }
>(function AudioPlayer({ src, label, durationMs, onSave }, ref) {
  const { t } = useI18n();
  const media = useRef<HTMLAudioElement>(null);
  useImperativeHandle(ref, () => media.current!, []);
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(durationMs / 1000);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [rate, setRate] = useState(1);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [notice, setNotice] = useState('');
  const saveFlight = useRef(false);
  useEffect(() => {
    setPlaying(false);
    setPosition(0);
    setDuration(durationMs / 1000);
    setReady(false);
    setError('');
    setNotice('');
  }, [src, durationMs]);
  function sync() {
    const audio = media.current;
    if (!audio) return;
    setPlaying(!audio.paused && !audio.ended);
    setPosition(audio.currentTime);
    setDuration(
      Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : durationMs / 1000,
    );
    setReady(audio.readyState >= 1);
    setVolume(audio.volume);
    setMuted(audio.muted);
    setRate(audio.playbackRate);
  }
  async function toggle() {
    const audio = media.current;
    if (!audio) return;
    setError('');
    if (!audio.paused) audio.pause();
    else
      try {
        await audio.play();
      } catch {
        setError(
          t(
            '无法播放这段音频。请重试，或保存原音频后打开。',
            'Could not play this recording. Retry or save the original audio to open it.',
          ),
        );
      }
  }
  async function save() {
    if (saveFlight.current) return;
    saveFlight.current = true;
    setSaving(true);
    setNotice('');
    setError('');
    try {
      if (await onSave()) setNotice(t('原音频已保存。', 'Original audio saved.'));
    } catch {
      setError(
        t(
          '保存原音频失败，请检查保存位置后重试。',
          'Could not save the original audio. Check the destination and retry.',
        ),
      );
    } finally {
      saveFlight.current = false;
      setSaving(false);
    }
  }
  return (
    <div className="ui-audio-player" role="group" aria-label={label}>
      <audio
        ref={media}
        src={src}
        preload="metadata"
        hidden
        onLoadedMetadata={sync}
        onDurationChange={sync}
        onTimeUpdate={sync}
        onPlay={sync}
        onPause={sync}
        onEnded={sync}
        onVolumeChange={sync}
        onRateChange={sync}
        onError={() => {
          setReady(false);
          setError(
            t(
              '音频无法读取。可保存原音频后重试。',
              'The audio could not be read. Save the original recording and retry.',
            ),
          );
        }}
      />
      <div className="ui-audio-controls">
        <button
          type="button"
          className="icon-button"
          aria-label={playing ? t('暂停播放', 'Pause playback') : t('播放录音', 'Play recording')}
          onClick={() => void toggle()}
        >
          {playing ? <Pause size={17} /> : <Play size={17} />}
        </button>
        <input
          className="ui-audio-seek"
          type="range"
          min={0}
          max={Math.max(duration, 0.1)}
          step={0.1}
          value={Math.min(position, duration)}
          disabled={!ready}
          aria-label={t('播放进度', 'Playback position')}
          aria-valuetext={`${formatDuration(position * 1000)} / ${formatDuration(duration * 1000)}`}
          onChange={(event) => {
            if (media.current) {
              media.current.currentTime = Number(event.target.value);
              sync();
            }
          }}
        />
        <span className="ui-audio-time">
          {formatDuration(position * 1000)} / {formatDuration(duration * 1000)}
        </span>
        <div className="ui-audio-volume">
          <button
            type="button"
            className="icon-button"
            aria-label={muted ? t('取消静音', 'Unmute') : t('静音', 'Mute')}
            onClick={() => {
              if (media.current) {
                media.current.muted = !media.current.muted;
                sync();
              }
            }}
          >
            {muted || volume === 0 ? <VolumeX size={17} /> : <Volume2 size={17} />}
          </button>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={muted ? 0 : volume}
            aria-label={t('播放音量', 'Playback volume')}
            aria-valuetext={`${Math.round((muted ? 0 : volume) * 100)}%`}
            onChange={(event) => {
              if (media.current) {
                media.current.muted = false;
                media.current.volume = Number(event.target.value);
                sync();
              }
            }}
          />
        </div>
        <PopupMenu
          label={t('更多播放操作', 'More playback options')}
          trigger={<MoreHorizontal size={18} />}
          placement="bottom-start"
          items={[
            ...[0.5, 0.75, 1, 1.25, 1.5, 2].map((value, index) => ({
              id: `rate-${value}`,
              label: value === 1 ? t('正常速度', 'Normal speed') : `${value}×`,
              groupLabel: index === 0 ? t('播放速度', 'Playback speed') : undefined,
              radio: true,
              checked: rate === value,
              onSelect: () => {
                if (media.current) {
                  media.current.playbackRate = value;
                  sync();
                }
              },
            })),
            {
              id: 'save',
              label: saving ? t('正在保存…', 'Saving…') : t('保存原音频', 'Save original audio'),
              icon: <Download size={16} />,
              disabled: saving,
              separatorBefore: true,
              onSelect: () => void save(),
            },
          ]}
        />
      </div>
      {notice && (
        <p className="ui-audio-notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <p className="ui-audio-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
});

import { Select } from '../ui/Select';
import { AudioPlayer } from '../ui/AudioPlayer';
import { saveVoiceAudio } from './api';
import {
  AlertTriangle,
  ArrowLeft,
  Ellipsis,
  Search,
  CheckCircle2,
  Clock3,
  Download,
  ExternalLink,
  FileAudio,
  Loader2,
  Mic,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Save,
  Settings2,
  Sparkles,
  Square,
  Trash2,
  Upload,
  Users,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  AsrSettingsState,
  CollectionDetail,
  CreateVoiceCapture,
  VoiceCapture,
  VoiceSyncTask,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call } from '../api';
import { displayDateTime } from '../format';
import { useI18n } from '../i18n';
import { PageFrame } from '../layout/PageFrame';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { PopupMenu } from '../ui/PopupMenu';
import { voiceErrorText, voiceErrorTextByCode } from './voice-errors';
import { PageState } from '../layout/PageState';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import {
  createVoiceCapture,
  getAsrSettings,
  importVoiceSync,
  browseVoiceCaptures,
  getVoiceCapture,
  listVoiceSyncTasks,
  loadVoiceAudio,
  saveVoiceCaptureNote,
  summarizeVoiceCapture,
  transcribeVoiceCapture,
  trashVoiceCapture,
  updateVoiceCapture,
} from './api';
import {
  appendDraftChunk,
  deleteDraft,
  draftDurationMs,
  listDrafts,
  loadDraft,
  startDraft,
  type VoiceDraft,
} from './drafts';
import {
  base64FromBlob,
  decodeWaveform,
  formatDuration,
  levelFromWaveform,
  suggestCaptureTitle,
  toSrt,
  toVtt,
  transcriptCues,
  transcriptPreview,
} from './voice-model';
import {
  type ActiveRecording,
  type AudioInput,
  RecorderError,
  listAudioInputs,
  readAudioDuration,
  startMeetingRecording,
  startRecording,
} from './recorder';
import './voice-capture.css';

const LOCAL_MAX_MS = 30 * 60 * 1000;
const IMPORT_MAX_MS = 4 * 60 * 60 * 1000;
const MAX_AUDIO_BYTES = 256 * 1024 * 1024;

type Notice = { tone: 'status' | 'error' | 'success'; text: string };

export function VoiceCapturePage({
  identity,
  onOpenNote,
  onOpenAsrSettings,
  onRegisterSave,
  settingsRevision,
}: {
  identity: WorkspaceIdentity;
  onOpenNote: (noteId: string) => void;
  onOpenAsrSettings: () => void;
  onRegisterSave: (save: () => Promise<void>) => () => void;
  settingsRevision: number;
}) {
  const { t, language } = useI18n();
  const [view, setView] = usePageNavigationState<'active' | 'trashed'>('voice.mode', 'active');
  const [selectedId, setSelectedId] = usePageNavigationState<string | null>('voice.selected', null);
  const [query, setQuery] = usePageNavigationState('voice.query', '');
  const [savedList, setSavedList] = usePageNavigationState('voice.position', {
    scope: '',
    page: 1,
    size: 10,
    scroll: 0,
  });
  const [selectedSnapshot, setSelectedSnapshot] = useState<VoiceCapture | null>(null);
  const [selectedError, setSelectedError] = useState('');
  const listViewport = useRef<HTMLUListElement>(null);
  const restoredList = useRef(false);
  const restoringList = useRef(false);
  const [restoreTick, setRestoreTick] = useState(0);
  const editorBase = useRef<{ id: string; row: number } | null>(null);
  const saveFlight = useRef<Promise<VoiceCapture | null> | null>(null);
  const [title, setTitle] = useState('');
  const [transcript, setTranscript] = useState('');
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState<
    null | 'create' | 'save' | 'transcribe' | 'note' | 'trash' | 'audio'
  >(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [audioError, setAudioError] = useState('');
  const [asr, setAsr] = useState<AsrSettingsState | null>(null);
  const [asrError, setAsrError] = useState('');
  const [autoTranscribe, setAutoTranscribe] = useState(true);
  const [confirm, setConfirm] = useState<null | 'trash' | 'restore' | 'retranscribe' | 'reload'>(
    null,
  );
  const [recording, setRecording] = useState<ActiveRecording | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [level, setLevel] = useState(0);
  const [paused, setPaused] = useState(false);
  const [meetingMode, setMeetingMode] = useState(false);
  const [devices, setDevices] = useState<AudioInput[]>([]);
  const [deviceId, setDeviceId] = useState('');
  const [peaks, setPeaks] = useState<number[]>([]);
  const [summary, setSummary] = useState<{ mode: 'summary' | 'actions'; text: string } | null>(
    null,
  );
  const [summaryBusy, setSummaryBusy] = useState(false);
  const [syncBusy, setSyncBusy] = useState(false);
  const [syncTasks, setSyncTasks] = useState<VoiceSyncTask[]>([]);
  const [drafts, setDrafts] = useState<VoiceDraft[]>([]);
  const [draftBusy, setDraftBusy] = useState<string | null>(null);
  const [collections, setCollections] = useState<CollectionDetail[]>([]);
  const [collectionId, setCollectionId] = useState('');
  const [tagText, setTagText] = useState('');
  const [recorderError, setRecorderError] = useState('');
  const importRef = useRef<HTMLInputElement>(null);
  const audioRef = useRef<HTMLAudioElement>(null);
  const analyserCleanup = useRef<(() => void) | null>(null);
  const stopRequested = useRef(false);
  const draftId = useRef<string | null>(null);

  const listScope = JSON.stringify([
    identity.local_instance_id,
    identity.workspace_id,
    view,
    query.trim(),
  ]);
  const voiceRows = usePagedList<VoiceCapture>(
    listScope,
    async (cursor, size) => {
      const response = await browseVoiceCaptures(identity, {
        limit: size,
        page: cursor ? Number(cursor) : 1,
        trashed: view === 'trashed',
        query: query.trim() || null,
      });
      return { ...response, page_index: response.page };
    },
    savedList.size,
    true,
  );
  const rowsRef = useRef(voiceRows);
  rowsRef.current = voiceRows;
  const page = voiceRows.page;
  const listState =
    voiceRows.loading || (!page && !voiceRows.error)
      ? 'loading'
      : voiceRows.error
        ? 'error'
        : 'ready';
  const listError = voiceRows.error ?? '';
  const selected = useMemo(
    () =>
      page?.items.find((item) => item.id === selectedId) ??
      (selectedSnapshot?.id === selectedId ? selectedSnapshot : null),
    [page, selectedId, selectedSnapshot],
  );
  useEffect(() => {
    let live = true;
    setSelectedError('');
    if (!selectedId) {
      setSelectedSnapshot(null);
      return;
    }
    if (page?.items.some((item) => item.id === selectedId)) return;
    void getVoiceCapture(identity, selectedId)
      .then((capture) => {
        if (live) setSelectedSnapshot(capture);
      })
      .catch((failure) => {
        if (live) setSelectedError(voiceErrorText(failure, t));
      });
    return () => {
      live = false;
    };
  }, [identity, selectedId, page]);
  const loadList = useCallback(
    async (preferredId?: string | null) => {
      if (preferredId !== undefined) setSelectedId(preferredId);
      setSelectedSnapshot(null);
      await rowsRef.current.refresh();
    },
    [setSelectedId],
  );
  useEffect(() => {
    if (restoredList.current || restoringList.current || voiceRows.loading || !page) return;
    if (savedList.scope !== listScope) {
      restoredList.current = true;
      return;
    }
    if (voiceRows.index !== savedList.page) {
      restoringList.current = true;
      void voiceRows.goTo(savedList.page).finally(() => {
        restoringList.current = false;
        restoredList.current = true;
        setRestoreTick((value) => value + 1);
      });
      return;
    }
    restoredList.current = true;
  }, [voiceRows, page, listScope, savedList]);
  useEffect(() => {
    if (
      !selectedId &&
      restoredList.current &&
      !voiceRows.loading &&
      savedList.scope === listScope &&
      listViewport.current
    )
      listViewport.current.scrollTop = savedList.scroll;
  }, [selectedId, page, restoreTick]);
  function rememberList() {
    setSavedList({
      scope: listScope,
      page: voiceRows.index,
      size: voiceRows.size,
      scroll: listViewport.current?.scrollTop ?? 0,
    });
  }
  async function backToRecordings() {
    if (dirty && !(await persistText())) return;
    setSelectedId(null);
    setSelectedSnapshot(null);
    setNotice(null);
  }

  const loadAsr = useCallback(async () => {
    setAsrError('');
    try {
      setAsr(await getAsrSettings(identity));
    } catch (error) {
      setAsrError(voiceErrorText(error, t));
    }
  }, [identity, t]);

  const refreshSyncTasks = useCallback(async () => {
    try {
      setSyncTasks(await listVoiceSyncTasks(identity));
    } catch {
      setSyncTasks([]);
    }
  }, [identity]);

  useEffect(() => {
    void refreshSyncTasks();
  }, [refreshSyncTasks]);

  useEffect(() => {
    void loadAsr();
  }, [loadAsr, settingsRevision]);

  useEffect(() => {
    void listAudioInputs()
      .then(setDevices)
      .catch(() => undefined);
    void listDrafts()
      .then(setDrafts)
      .catch(() => undefined);
    void call(
      {
        kind: 'collections_list',
        payload: {
          limit: 100,
          cursor: null,
          trashed: false,
          archived: false,
          pinned_only: false,
          query: null,
        },
      },
      'collections',
      identity,
    )
      .then((page) => setCollections(page.items))
      .catch(() => setCollections([]));
  }, [identity]);

  useEffect(() => {
    if (!selected) {
      if (!selectedId) {
        setTitle('');
        setTranscript('');
        setDirty(false);
        editorBase.current = null;
      }
      return;
    }
    if (dirty && editorBase.current?.id === selected.id) {
      if (editorBase.current.row !== selected.row_version)
        setNotice({
          tone: 'error',
          text: t(
            '录音已在别处更新；本地修改已保留，可复制文本或重新载入。',
            'This recording changed elsewhere. Your local text is kept; copy it or reload.',
          ),
        });
      return;
    }
    editorBase.current = { id: selected.id, row: selected.row_version };
    setTitle(selected.title);
    setTranscript(selected.transcript);
    setDirty(false);
  }, [selected?.id, selected?.row_version, selectedId]);

  useEffect(() => {
    if (!selected) {
      setAudioUrl(null);
      setAudioError('');
      return;
    }
    let cancelled = false;
    let url: string | null = null;
    setBusy((current) => current ?? 'audio');
    setAudioError('');
    void loadVoiceAudio(identity, selected.id)
      .then((bytes) => {
        if (cancelled) return;
        url = URL.createObjectURL(new Blob([bytes], { type: selected.audio_media_type }));
        setAudioUrl(url);
      })
      .catch((error) => {
        if (!cancelled) setAudioError(voiceErrorText(error, t));
      })
      .finally(() => {
        if (!cancelled) setBusy((current) => (current === 'audio' ? null : current));
      });
    return () => {
      cancelled = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [identity, selected?.id, selected?.row_version, selected?.audio_media_type, t]);

  useEffect(() => {
    if (!recording) return;
    const timer = window.setInterval(() => {
      const value = recording.elapsedMs();
      setElapsed(value);
      if (value >= LOCAL_MAX_MS && !stopRequested.current) {
        stopRequested.current = true;
        void stopRecording();
      }
    }, 200);
    return () => window.clearInterval(timer);
    // stopRecording is stable for the lifetime of one recording.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording]);

  useEffect(() => {
    if (!recording) return;
    // Release only the analyser owned by this recording. The previous empty
    // effect must not read a newly attached analyser from the shared ref.
    const releaseAnalyser = analyserCleanup.current;
    return () => {
      releaseAnalyser?.();
      if (analyserCleanup.current === releaseAnalyser) analyserCleanup.current = null;
      recording.cancel();
    };
  }, [recording]);

  function applyCapture(capture: VoiceCapture) {
    rowsRef.current.updateItems((items) =>
      items.map((item) => (item.id === capture.id ? capture : item)),
    );
    setSelectedSnapshot((current) => (current?.id === capture.id ? capture : current));
  }

  async function startCapture() {
    setRecorderError('');
    setNotice(null);
    const draft = crypto.randomUUID();
    try {
      await startDraft(draft, 'audio/webm').catch(() => undefined);
      const active = await startRecording({
        deviceId: deviceId || null,
        onChunk: (chunk) => {
          void appendDraftChunk(draft, chunk).catch(() => undefined);
        },
      });
      draftId.current = draft;
      setElapsed(0);
      setLevel(0);
      setPaused(false);
      setRecording(active);
      stopRequested.current = false;
      attachAnalyser(active.stream);
      // Labels become available only after permission has been granted.
      void listAudioInputs()
        .then(setDevices)
        .catch(() => undefined);
    } catch (error) {
      await deleteDraft(draft).catch(() => undefined);
      setRecorderError(recorderErrorText(error, t));
    }
  }

  async function startMeetingCapture() {
    setRecorderError('');
    setNotice(null);
    try {
      const active = await startMeetingRecording({ deviceId: deviceId || null });
      setElapsed(0);
      setLevel(0);
      setPaused(false);
      setMeetingMode(true);
      setRecording(active);
      stopRequested.current = false;
      attachAnalyser(active.stream);
    } catch (error) {
      setRecorderError(recorderErrorText(error, t));
    }
  }

  function attachAnalyser(stream: MediaStream) {
    try {
      const context = new AudioContext();
      const source = context.createMediaStreamSource(stream);
      const analyser = context.createAnalyser();
      analyser.fftSize = 512;
      source.connect(analyser);
      const buffer = new Uint8Array(analyser.fftSize);
      let frame = 0;
      let cancelled = false;
      const tick = () => {
        if (cancelled) return;
        analyser.getByteTimeDomainData(buffer);
        setLevel(levelFromWaveform(buffer));
        frame = requestAnimationFrame(tick);
      };
      tick();
      analyserCleanup.current = () => {
        if (cancelled) return;
        cancelled = true;
        cancelAnimationFrame(frame);
        source.disconnect();
        analyser.disconnect();
        void context.close();
      };
    } catch {
      // The level meter is optional; recording and saving still work.
      analyserCleanup.current = null;
    }
  }

  async function recoverDraft(draft: VoiceDraft) {
    setDraftBusy(draft.id);
    setRecorderError('');
    try {
      const blob = await loadDraft(draft.id);
      if (!blob || blob.size === 0) {
        await deleteDraft(draft.id);
        setDrafts(await listDrafts());
        setRecorderError(
          t('这段草稿里没有可用音频。', 'This draft contains no recoverable audio.'),
        );
        return;
      }
      const audioBase64 = await base64FromBlob(blob);
      const capture = await createVoiceCapture(
        identity,
        {
          title: t('恢复的录音', 'Recovered recording'),
          duration_ms: draftDurationMs(draft),
          source_kind: 'local',
          device_label: null,
          language: null,
        },
        audioBase64,
      );
      await deleteDraft(draft.id);
      setDrafts(await listDrafts());
      applyCapture(capture);
      setView('active');
      setNotice({
        tone: 'success',
        text: t('未完成录音已恢复并保存。', 'The unfinished recording was recovered and saved.'),
      });
      if (autoTranscribe && asr?.credential_configured && asr.settings.endpoint) {
        await runTranscription(capture);
      }
    } catch (error) {
      setRecorderError(voiceErrorText(error, t));
    } finally {
      setDraftBusy(null);
    }
  }

  async function discardDraft(draft: VoiceDraft) {
    setDraftBusy(draft.id);
    try {
      await deleteDraft(draft.id);
      setDrafts(await listDrafts());
    } catch {
      setRecorderError(t('无法删除这段草稿。', 'The draft could not be removed.'));
    } finally {
      setDraftBusy(null);
    }
  }

  function togglePause() {
    if (!recording) return;
    if (paused) {
      recording.resume();
      setPaused(false);
    } else {
      recording.pause();
      setPaused(true);
    }
  }

  async function stopRecording() {
    if (!recording || stopRequested.current) return;
    stopRequested.current = true;
    setBusy('create');
    analyserCleanup.current?.();
    analyserCleanup.current = null;
    const result = await recording.stop();
    setRecording(null);
    setElapsed(0);
    setLevel(0);
    setPaused(false);
    if (result.blob.size === 0) {
      if (draftId.current) await deleteDraft(draftId.current).catch(() => undefined);
      draftId.current = null;
      setRecorderError(
        t(
          '没有录到音频，请检查麦克风后重试。',
          'No audio was captured. Check the microphone and try again.',
        ),
      );
      setBusy(null);
      return;
    }
    try {
      const audioBase64 = await base64FromBlob(result.blob);
      const request: CreateVoiceCapture = {
        title: meetingMode ? t('会议录音', 'Meeting recording') : '',
        duration_ms: result.durationMs,
        source_kind: 'local',
        device_label: meetingMode ? t('麦克风 + 系统声音', 'Microphone + system audio') : null,
        language: null,
      };
      const capture = await createVoiceCapture(identity, request, audioBase64);
      if (draftId.current) {
        await deleteDraft(draftId.current).catch(() => undefined);
        draftId.current = null;
        void listDrafts()
          .then(setDrafts)
          .catch(() => undefined);
      }
      applyCapture(capture);
      setView('active');
      setNotice({
        tone: 'status',
        text: t(
          '录音已保存。原始音频不会因转写失败而丢失。',
          'Recording saved. The original audio is preserved even if transcription fails.',
        ),
      });
      if (autoTranscribe && asr?.credential_configured && asr.settings.endpoint) {
        await runTranscription(capture);
      }
    } catch (error) {
      setRecorderError(voiceErrorText(error, t));
    } finally {
      setBusy(null);
      stopRequested.current = false;
      setMeetingMode(false);
    }
  }

  async function importAudio(file: File) {
    setNotice(null);
    setRecorderError('');
    if (file.size > MAX_AUDIO_BYTES) {
      setRecorderError(t('音频超过 256 MiB，无法导入。', 'The audio is larger than 256 MiB.'));
      return;
    }
    setBusy('create');
    try {
      const duration = await readAudioDuration(file);
      if (duration > IMPORT_MAX_MS) {
        setRecorderError(t('音频超过 4 小时，无法导入。', 'The audio is longer than 4 hours.'));
        return;
      }
      const audioBase64 = await base64FromBlob(file);
      const cleanTitle = file.name.replace(/\.[^.]+$/, '').slice(0, 120);
      const capture = await createVoiceCapture(
        identity,
        {
          title: cleanTitle,
          duration_ms: duration,
          source_kind: 'import',
          device_label: null,
          language: null,
        },
        audioBase64,
      );
      applyCapture(capture);
      setView('active');
      setNotice({
        tone: 'status',
        text: t('音频已原样保存。', 'The audio was saved unchanged.'),
      });
      if (autoTranscribe && asr?.credential_configured && asr.settings.endpoint) {
        await runTranscription(capture);
      }
    } catch (error) {
      setRecorderError(voiceErrorText(error, t));
    } finally {
      setBusy(null);
      if (importRef.current) importRef.current.value = '';
    }
  }

  useEffect(() => {
    if (!audioUrl || !selected || selected.audio_bytes > 60 * 1024 * 1024) {
      setPeaks([]);
      return;
    }
    let cancelled = false;
    void fetch(audioUrl)
      .then((response) => response.blob())
      .then((blob) => decodeWaveform(blob, 72))
      .then((values) => {
        if (!cancelled) setPeaks(values);
      })
      .catch(() => {
        if (!cancelled) setPeaks([]);
      });
    return () => {
      cancelled = true;
    };
  }, [audioUrl, selected?.id, selected?.audio_bytes]);

  function exportSubtitles(format: 'srt' | 'vtt') {
    if (!selected) return;
    const cues: { startMs: number; endMs: number; text: string }[] = selected.segments.length
      ? selected.segments.map((segment) => ({
          startMs: segment.start_ms,
          endMs: segment.end_ms,
          text: segment.text,
        }))
      : transcriptCues(transcript, selected.duration_ms);
    if (cues.length === 0) return;
    const content = format === 'srt' ? toSrt(cues) : toVtt(cues);
    const blob = new Blob([content], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `${selected.title.replace(/[\\/:*?"<>|]/g, '_') || 'voice'}.${format}`;
    anchor.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 2_000);
  }

  async function generateSummary(mode: 'summary' | 'actions') {
    if (!selected) return;
    setSummaryBusy(true);
    setSummary(null);
    try {
      const result = await summarizeVoiceCapture(identity, selected.id, mode);
      setSummary({ mode, text: result.text });
    } catch (error) {
      setNotice({ tone: 'error', text: voiceErrorText(error, t) });
    } finally {
      setSummaryBusy(false);
    }
  }

  async function pullSyncedVoice() {
    setSyncBusy(true);
    try {
      // Pull the marker note first; import is a separate deterministic step.
      await call({ kind: 'sync_pull' }, 'sync_pull', identity).catch(() => undefined);
      const result = await importVoiceSync(identity);
      setNotice({
        tone: result.failed.length > 0 ? 'error' : 'success',
        text:
          result.failed.length > 0
            ? t(
                `已导入 ${result.imported} 条，失败 ${result.failed.length} 条：${result.failed[0]}`,
                `Imported ${result.imported}; ${result.failed.length} failed: ${result.failed[0]}`,
              )
            : t(
                `已从手机同步导入 ${result.imported} 条语音。`,
                `Imported ${result.imported} voice capture(s) from sync.`,
              ),
      });
      await loadList();
      await refreshSyncTasks();
    } catch (error) {
      setNotice({ tone: 'error', text: voiceErrorText(error, t) });
    } finally {
      setSyncBusy(false);
    }
  }

  function persistText(): Promise<VoiceCapture | null> {
    if (saveFlight.current) return saveFlight.current;
    const operation = persistTextNow();
    saveFlight.current = operation;
    void operation.finally(() => {
      if (saveFlight.current === operation) saveFlight.current = null;
    });
    return operation;
  }
  async function persistTextNow(): Promise<VoiceCapture | null> {
    if (!selected) return null;
    if (!title.trim()) {
      setNotice({ tone: 'error', text: t('标题不能为空。', 'The title cannot be empty.') });
      return null;
    }
    setBusy('save');
    try {
      const updated = await updateVoiceCapture(identity, {
        capture_id: selected.id,
        base_row_version:
          editorBase.current?.id === selected.id ? editorBase.current.row : selected.row_version,
        title: title.trim(),
        transcript,
      });
      applyCapture(updated);
      setDirty(false);
      setNotice({ tone: 'success', text: t('转写文本已保存。', 'Transcript saved.') });
      return updated;
    } catch (error) {
      setNotice({ tone: 'error', text: voiceErrorText(error, t) });
      return null;
    } finally {
      setBusy(null);
    }
  }

  async function runTranscription(target?: VoiceCapture) {
    let capture = target ?? selected;
    if (!capture) return;
    if (!asr?.credential_configured || !asr.settings.endpoint) {
      setNotice({
        tone: 'error',
        text: t(
          '尚未配置转写服务。音频已保存，可先手工输入文本。',
          'No transcription service is configured. The audio is saved; you can type the text manually.',
        ),
      });
      return;
    }
    if (!target && dirty) {
      const saved = await persistText();
      if (!saved) return;
      capture = saved;
    }
    setBusy('transcribe');
    setNotice({ tone: 'status', text: t('正在转写…', 'Transcribing…') });
    try {
      const updated = await transcribeVoiceCapture(identity, capture.id, capture.row_version);
      applyCapture(updated);
      setDirty(false);
      setNotice({
        tone: 'success',
        text: t(
          '转写完成。请检查文本后再存入收件箱。',
          'Transcription finished. Review the text before saving it to the inbox.',
        ),
      });
    } catch (error) {
      setNotice({ tone: 'error', text: voiceErrorText(error, t) });
      // The kernel records a failed attempt; reload to show its error code
      // without guessing whether the audio is still intact.
      await loadList(capture.id);
    } finally {
      setBusy(null);
    }
  }

  async function promoteToInbox() {
    if (!selected) return;
    let current = selected;
    if (dirty) {
      const saved = await persistText();
      if (!saved) return;
      current = saved;
    }
    if (!current.transcript.trim()) {
      setNotice({
        tone: 'error',
        text: t(
          '先转写或手工输入文本，再存入收件箱。',
          'Transcribe or type the text before saving it to the inbox.',
        ),
      });
      return;
    }
    setBusy('note');
    try {
      const collection = collections.find((item) => item.id === collectionId);
      const tags = tagText
        .split(/[,，\s]+/)
        .map((tag) => tag.trim())
        .filter(Boolean)
        .slice(0, 12);
      const updated = await saveVoiceCaptureNote(identity, {
        capture_id: current.id,
        base_row_version: current.row_version,
        collection_id: collectionId || undefined,
        base_collection_row_version: collection?.metadata.row_version,
        tag_names: tags,
      });
      applyCapture(updated);
      setNotice({
        tone: 'success',
        text: t('已存入收件箱。', 'Saved to the inbox.'),
      });
    } catch (error) {
      setNotice({ tone: 'error', text: voiceErrorText(error, t) });
    } finally {
      setBusy(null);
    }
  }

  async function changeTrash(trashed: boolean) {
    if (!selected) return;
    setBusy('trash');
    try {
      const updated = await trashVoiceCapture(identity, {
        capture_id: selected.id,
        base_row_version: selected.row_version,
        trashed,
      });
      setConfirm(null);
      setNotice({
        tone: 'success',
        text: trashed
          ? t(
              '已移入语音回收站，原始音频仍保留。',
              'Moved to the voice trash; the original audio is kept.',
            )
          : t('已恢复。', 'Restored.'),
      });
      if (!trashed) setView('active');
      await loadList(trashed ? null : updated.id);
    } catch (error) {
      setNotice({ tone: 'error', text: voiceErrorText(error, t) });
    } finally {
      setBusy(null);
    }
  }

  async function reloadText() {
    if (!selected || busy) return;
    setBusy('save');
    try {
      const capture = await getVoiceCapture(identity, selected.id);
      rowsRef.current.updateItems((items) =>
        items.map((item) => (item.id === capture.id ? capture : item)),
      );
      setSelectedSnapshot(capture);
      editorBase.current = { id: capture.id, row: capture.row_version };
      setTitle(capture.title);
      setTranscript(capture.transcript);
      setDirty(false);
      setConfirm(null);
      setNotice(null);
    } catch (failure) {
      setNotice({ tone: 'error', text: voiceErrorText(failure, t) });
    } finally {
      setBusy(null);
    }
  }

  const leaveRef = useRef<() => Promise<void>>(async () => {});
  leaveRef.current = async () => {
    if (recording)
      throw new Error(
        t(
          '正在录音，请先停止并保存后再切换页面。',
          'Stop and save the recording before leaving this page.',
        ),
      );
    if (busy === 'create')
      throw new Error(
        t(
          '正在保存录音，请稍后再切换页面。',
          'The recording is being saved. Please wait before leaving.',
        ),
      );
    if ((dirty || saveFlight.current) && !(await persistText()))
      throw new Error(
        t(
          '语音文本尚未保存，页面已保留，请处理保存错误后重试。',
          'The voice text was not saved. This page is kept; resolve the save error and retry.',
        ),
      );
  };
  useEffect(() => onRegisterSave(() => leaveRef.current()), [onRegisterSave]);

  const status = selected ? statusLabel(selected.status, t) : null;
  const canTranscribe = !!selected && !selected.trashed;
  const canPromote = !!selected && !selected.trashed && !selected.note_id;

  return (
    <PageFrame
      className={`voice-page ${selectedId ? 'voice-content-page' : 'voice-library-page'}`}
      title={selectedId ? t('语音内容', 'Recording') : t('语音速记', 'Voice capture')}
      toolbar={
        !selectedId || recording ? (
          <section className="voice-recorder" aria-label={t('录音', 'Recording')}>
            <button
              type="button"
              className={`voice-record-button ${recording ? 'is-recording' : ''}`}
              aria-pressed={recording !== null}
              aria-label={
                recording
                  ? t('停止并保存录音', 'Stop and save recording')
                  : t('开始录音', 'Start recording')
              }
              disabled={busy !== null && busy !== 'create'}
              onClick={() => (recording ? void stopRecording() : void startCapture())}
            >
              {recording ? (
                <Square size={24} aria-hidden="true" />
              ) : (
                <Mic size={26} aria-hidden="true" />
              )}
            </button>
            <div className="voice-recorder-copy">
              <strong>
                {recording
                  ? paused
                    ? t('录音已暂停', 'Recording paused')
                    : t('正在录音', 'Recording')
                  : t('按一下开始口述', 'Tap to start speaking')}
              </strong>
              <span className="voice-recorder-timer" aria-live="off">
                {formatDuration(recording ? elapsed : 0)}
                <span className="voice-recorder-limit">
                  {t(' / 最长 30 分钟', ' / 30 min max')}
                </span>
              </span>
              <div className="voice-level" aria-hidden="true">
                <span style={{ width: `${Math.round(level * 100)}%` }} />
              </div>

              {!recording && devices.length > 0 && (
                <div
                  className="voice-device"
                  role="group"
                  aria-label={t('麦克风设备', 'Microphone device')}
                >
                  <span>{t('麦克风', 'Microphone')}</span>
                  <Select
                    aria-label={t('麦克风', 'Microphone')}
                    value={deviceId}
                    disabled={busy !== null}
                    onValueChange={(value) => setDeviceId(value)}
                  >
                    <option value="">{t('系统默认', 'System default')}</option>
                    {devices.map((device) => (
                      <option key={device.deviceId} value={device.deviceId}>
                        {device.label}
                      </option>
                    ))}
                  </Select>
                </div>
              )}
              <label className="voice-checkbox voice-auto-transcribe">
                <input
                  type="checkbox"
                  checked={autoTranscribe}
                  onChange={(event) => setAutoTranscribe(event.target.checked)}
                />
                {t('完成后自动转写', 'Transcribe when finished')}
              </label>
            </div>
            <div className="voice-recorder-actions">
              <button
                type="button"
                className="secondary-button"
                disabled={recording !== null || busy !== null}
                onClick={() => importRef.current?.click()}
              >
                <Upload size={15} aria-hidden="true" />
                {t('导入音频', 'Import audio')}
              </button>
              {!recording && (
                <button
                  type="button"
                  className="secondary-button"
                  disabled={busy !== null}
                  onClick={() => void startMeetingCapture()}
                  title={t(
                    '同时录制麦克风和系统声音',
                    'Record microphone and system audio together',
                  )}
                >
                  <Users size={15} aria-hidden="true" />
                  {t('会议录音', 'Meeting')}
                </button>
              )}
              {recording && (
                <>
                  <button type="button" className="secondary-button" onClick={() => togglePause()}>
                    {paused ? (
                      <Play size={15} aria-hidden="true" />
                    ) : (
                      <Pause size={15} aria-hidden="true" />
                    )}
                    {paused ? t('继续', 'Resume') : t('暂停', 'Pause')}
                  </button>
                  <button
                    type="button"
                    className="text-button"
                    onClick={() => void stopRecording()}
                  >
                    {t('停止', 'Stop')}
                  </button>
                </>
              )}
            </div>
            {recorderError && (
              <p className="voice-inline-error" role="alert">
                <AlertTriangle size={15} aria-hidden="true" />
                {recorderError}
              </p>
            )}
          </section>
        ) : undefined
      }
      footer={
        selectedId ? (
          <div className="voice-actions voice-footer-actions">
            {canTranscribe && (
              <button
                type="button"
                className="secondary-button voice-footer-secondary"
                aria-label={t('转写', 'Transcribe')}
                disabled={busy !== null}
                onClick={() =>
                  dirty || selected?.transcript.trim()
                    ? setConfirm('retranscribe')
                    : void runTranscription()
                }
              >
                <Sparkles size={15} aria-hidden="true" />
                {t('转写', 'Transcribe')}
              </button>
            )}
            <button
              type="button"
              className="secondary-button voice-footer-secondary"
              aria-label={t('保存文本', 'Save text')}
              disabled={busy !== null || !dirty || selected?.trashed}
              onClick={() => void persistText()}
            >
              <Save size={15} aria-hidden="true" />
              {t('保存文本', 'Save text')}
            </button>
            {canPromote && (
              <button
                type="button"
                className="primary-button"
                disabled={busy !== null || !transcript.trim()}
                onClick={() => void promoteToInbox()}
              >
                <FileAudio size={15} aria-hidden="true" />
                {t('存入收件箱', 'Save to inbox')}
              </button>
            )}
            <PopupMenu
              label={t('更多录音操作', 'More recording actions')}
              trigger={<Ellipsis size={18} />}
              disabled={!selected || busy !== null}
              items={[
                {
                  id: 'summary',
                  label: t('AI 摘要', 'AI summary'),
                  icon: <Sparkles />,
                  disabled: summaryBusy || !transcript.trim(),
                  onSelect: () => void generateSummary('summary'),
                },
                {
                  id: 'actions',
                  label: t('提取待办', 'Extract actions'),
                  disabled: summaryBusy || !transcript.trim(),
                  onSelect: () => void generateSummary('actions'),
                },
                {
                  id: 'srt',
                  label: t('导出 SRT 字幕', 'Export SRT subtitles'),
                  icon: <Download />,
                  disabled: !transcript.trim(),
                  onSelect: () => exportSubtitles('srt'),
                },
                {
                  id: 'vtt',
                  label: t('导出 VTT 字幕', 'Export VTT subtitles'),
                  icon: <Download />,
                  disabled: !transcript.trim(),
                  onSelect: () => exportSubtitles('vtt'),
                },
                {
                  id: 'reload',
                  label: t('重新载入文本', 'Reload text'),
                  separatorBefore: true,
                  onSelect: () => setConfirm('reload'),
                },
                {
                  id: 'trash',
                  label: selected?.trashed
                    ? t('恢复录音', 'Restore recording')
                    : t('移到语音回收站', 'Move to voice trash'),
                  icon: <Trash2 />,
                  onSelect: () => setConfirm(selected?.trashed ? 'restore' : 'trash'),
                },
              ]}
            />
            <span className="voice-save-state" role="status">
              {busy === 'save'
                ? t('正在保存…', 'Saving…')
                : dirty
                  ? t('有未保存的修改', 'Unsaved changes')
                  : t('已保存', 'Saved')}
            </span>
          </div>
        ) : (
          <ListPagination list={voiceRows} />
        )
      }
      description={t(
        selectedId ? '查看录音、修订文本，再存入知识库。' : '录下想法，打开记录后转写或整理。',
        selectedId
          ? 'Listen, revise the text, then save it to your knowledge.'
          : 'Record an idea, then open it to transcribe or organize.',
      )}
      actions={
        <>
          {selectedId && (
            <button
              className="secondary-button"
              disabled={busy === 'save' || busy === 'create'}
              onClick={() => void backToRecordings()}
            >
              <ArrowLeft size={16} aria-hidden="true" />
              {t('返回录音列表', 'Back to recordings')}
            </button>
          )}
          <button
            className="secondary-button"
            disabled={Boolean(recording) || busy === 'create'}
            onClick={onOpenAsrSettings}
          >
            <Settings2 size={15} aria-hidden="true" />
            {asr?.credential_configured
              ? t('转写设置', 'Transcription settings')
              : t('配置转写服务', 'Configure transcription')}
          </button>
          {!selectedId && view === 'active' && (
            <button
              type="button"
              className="secondary-button"
              disabled={syncBusy || busy !== null}
              onClick={() => void pullSyncedVoice()}
            >
              {syncBusy ? (
                <Loader2 size={15} className="voice-spin" aria-hidden="true" />
              ) : (
                <Download size={15} aria-hidden="true" />
              )}
              {t('拉取手机语音', 'Pull phone audio')}
            </button>
          )}
          {!selectedId && (
            <button
              type="button"
              className="secondary-button"
              disabled={busy !== null}
              onClick={() => {
                setSelectedId(null);
                setView(view === 'active' ? 'trashed' : 'active');
                setNotice(null);
              }}
            >
              {view === 'active' ? (
                <Trash2 size={15} aria-hidden="true" />
              ) : (
                <RotateCcw size={15} aria-hidden="true" />
              )}
              {view === 'active'
                ? t('语音回收站', 'Voice trash')
                : t('返回录音', 'Back to recordings')}
            </button>
          )}
        </>
      }
    >
      <input
        ref={importRef}
        type="file"
        accept="audio/*,.webm,.ogg,.wav,.m4a,.mp3"
        className="voice-import-input"
        aria-label={t('选择音频文件', 'Choose an audio file')}
        onChange={(event) => {
          const file = event.currentTarget.files?.[0];
          if (file) void importAudio(file);
        }}
      />
      {drafts.length > 0 && !recording && (
        <section className="voice-drafts" aria-label={t('未完成的录音', 'Unfinished recordings')}>
          <header>
            <AlertTriangle size={15} aria-hidden="true" />
            <strong>{t('发现未完成的录音', 'Unfinished recordings found')}</strong>
            <span>
              {t(
                '上次录音没有正常保存，音频分片仍在本机。',
                'A previous recording did not finish saving; its chunks are still on this device.',
              )}
            </span>
          </header>
          <ul>
            {drafts.map((draft) => (
              <li key={draft.id}>
                <span>
                  {formatDuration(draftDurationMs(draft))} ·{' '}
                  {new Date(draft.startedAt).toLocaleTimeString(language)}
                </span>
                <button
                  type="button"
                  className="secondary-button"
                  disabled={draftBusy !== null}
                  onClick={() => void recoverDraft(draft)}
                >
                  {draftBusy === draft.id ? (
                    <Loader2 size={14} className="voice-spin" aria-hidden="true" />
                  ) : (
                    <RotateCcw size={14} aria-hidden="true" />
                  )}
                  {t('恢复', 'Recover')}
                </button>
                <button
                  type="button"
                  className="text-button is-danger"
                  disabled={draftBusy !== null}
                  onClick={() => void discardDraft(draft)}
                >
                  {t('丢弃', 'Discard')}
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {notice && (
        <p
          className={`voice-notice is-${notice.tone}`}
          role={notice.tone === 'error' ? 'alert' : 'status'}
        >
          {notice.text}
        </p>
      )}
      {syncTasks.length > 0 && (
        <section className="voice-sync-tasks" aria-label={t('语音同步任务', 'Voice sync tasks')}>
          <header>
            <strong>{t('语音同步', 'Voice sync')}</strong>
            <span className="voice-hint">
              {t('手机上传 → 桌面导入', 'Phone upload → desktop import')}
            </span>
          </header>
          <ul>
            {syncTasks.slice(0, 5).map((task) => (
              <li key={task.id}>
                <span className={`voice-task-status is-${task.status}`}>
                  {task.status === 'syncing'
                    ? t('同步中', 'Syncing')
                    : task.status === 'done'
                      ? t('成功', 'Done')
                      : task.status === 'failed'
                        ? t('失败', 'Failed')
                        : t('待同步', 'Pending')}
                </span>
                <code>{task.blob_hash.slice(0, 10)}…</code>
                {task.error_code && <span className="voice-hint">{task.error_code}</span>}
                {task.status === 'failed' && (
                  <button
                    type="button"
                    className="text-button"
                    disabled={syncBusy}
                    onClick={() => void pullSyncedVoice()}
                  >
                    {t('重试', 'Retry')}
                  </button>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
      {asrError && (
        <p className="voice-inline-error" role="alert">
          <AlertTriangle size={15} aria-hidden="true" />
          {asrError}
        </p>
      )}

      <div className={`voice-workspace ${selectedId ? 'is-detail' : 'is-list'}`}>
        {!selectedId && (
          <section className="voice-list" aria-label={t('录音列表', 'Voice recordings')}>
            <header className="voice-list-header">
              <h2>
                {view === 'active' ? t('待确认', 'To review') : t('语音回收站', 'Voice trash')}
              </h2>
              <button
                type="button"
                className="icon-button"
                aria-label={t('刷新录音列表', 'Refresh recordings')}
                disabled={listState === 'loading'}
                onClick={() => void loadList(selectedId)}
              >
                <RefreshCw size={15} aria-hidden="true" />
              </button>
            </header>
            <label className="voice-list-search">
              <Search size={16} aria-hidden="true" />
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                aria-label={t('搜索录音', 'Search recordings')}
                placeholder={t('搜索标题或转写文本…', 'Search titles or transcripts…')}
              />
            </label>
            {listState === 'error' ? (
              <PageState
                kind="error"
                compact
                title={t('无法读取录音', 'Could not load recordings')}
                detail={listError}
                retryLabel={t('重试', 'Retry')}
                onRetry={() => void loadList(selectedId)}
              />
            ) : listState === 'loading' && !page ? (
              <PageState kind="loading" compact title={t('正在读取录音…', 'Loading recordings…')} />
            ) : !page || page.items.length === 0 ? (
              <div className="voice-empty">
                <FileAudio size={22} aria-hidden="true" />
                <p>
                  {view === 'active'
                    ? t(
                        '还没有录音。按上面的按钮开始，或导入一个音频文件。',
                        'No recordings yet. Use the button above or import an audio file.',
                      )
                    : t('语音回收站是空的。', 'The voice trash is empty.')}
                </p>
              </div>
            ) : (
              <ul className="voice-list-items" ref={listViewport}>
                {page.items.map((capture) => (
                  <li key={capture.id}>
                    <button
                      type="button"
                      className={`voice-list-item ${capture.id === selectedId ? 'active' : ''}`}
                      aria-current={capture.id === selectedId ? 'true' : undefined}
                      onClick={() => {
                        rememberList();
                        setSelectedSnapshot(capture);
                        setSelectedId(capture.id);
                        setNotice(null);
                      }}
                    >
                      <span className="voice-list-item-top">
                        <strong>{capture.title || t('语音速记', 'Voice capture')}</strong>
                        <span className={`voice-status is-${capture.status}`}>
                          {statusLabel(capture.status, t)}
                        </span>
                      </span>
                      <span className="voice-list-item-meta">
                        <Clock3 size={12} aria-hidden="true" />
                        {formatDuration(capture.duration_ms)}
                        <span aria-hidden="true">·</span>
                        {displayDateTime(capture.created_at, language)}
                      </span>
                      <span className="voice-list-item-preview">
                        {capture.transcript
                          ? transcriptPreview(capture.transcript, 70)
                          : t('尚无转写文本', 'No transcript yet')}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        )}

        {selectedId && (
          <section className="voice-detail" aria-label={t('录音详情', 'Recording details')}>
            {!selected ? (
              selectedError ? (
                <PageState
                  kind="error"
                  title={t('无法读取录音', 'Could not load recording')}
                  detail={selectedError}
                  onRetry={() => void loadList(selectedId)}
                />
              ) : (
                <PageState kind="loading" title={t('正在读取录音…', 'Loading recording…')} />
              )
            ) : (
              <>
                <header className="voice-detail-header">
                  <label className="voice-field voice-title-field">
                    <span>{t('标题', 'Title')}</span>
                    <input
                      value={title}
                      maxLength={120}
                      disabled={selected.trashed || busy !== null}
                      onChange={(event) => {
                        setTitle(event.target.value);
                        setDirty(true);
                      }}
                    />
                  </label>
                  <div className="voice-detail-meta">
                    <span>
                      {selected.source_kind === 'local'
                        ? t('本机录音', 'Local recording')
                        : t('导入音频', 'Imported audio')}
                    </span>
                    <span aria-hidden="true">·</span>
                    <span>{formatDuration(selected.duration_ms)}</span>
                    <span aria-hidden="true">·</span>
                    <span>{displayDateTime(selected.created_at, language)}</span>
                    {status && (
                      <span className={`voice-status is-${selected.status}`}>{status}</span>
                    )}
                  </div>
                </header>

                {selected.note_id && (
                  <div className="voice-note-link">
                    <CheckCircle2 size={15} aria-hidden="true" />
                    <span>{t('已存入收件箱', 'Saved to the inbox')}</span>
                    <button
                      type="button"
                      className="text-button"
                      onClick={() => onOpenNote(selected.note_id!)}
                    >
                      {t('打开笔记', 'Open note')}
                      <ExternalLink size={13} aria-hidden="true" />
                    </button>
                  </div>
                )}

                <div className="voice-audio">
                  {audioError ? (
                    <p className="voice-inline-error" role="alert">
                      <AlertTriangle size={15} aria-hidden="true" />
                      {audioError}
                    </p>
                  ) : audioUrl ? (
                    <>
                      <AudioPlayer
                        ref={audioRef}
                        src={audioUrl}
                        durationMs={selected.duration_ms}
                        label={t('录音回放', 'Recording playback')}
                        onSave={() => saveVoiceAudio(identity, selected.id)}
                      />
                      {peaks.some((peak) => peak > 0.01) && (
                        <div className="voice-waveform" aria-hidden="true">
                          {peaks.map((peak, index) => (
                            <span
                              key={index}
                              style={{ height: `${Math.max(4, Math.round(peak * 100))}%` }}
                            />
                          ))}
                        </div>
                      )}
                    </>
                  ) : (
                    <p className="voice-hint" role="status">
                      <Loader2 size={14} className="voice-spin" aria-hidden="true" />
                      {t('正在读取音频…', 'Loading audio…')}
                    </p>
                  )}
                </div>

                {selected.error_code && (
                  <p className="voice-inline-error" role="alert">
                    <AlertTriangle size={15} aria-hidden="true" />
                    {voiceErrorTextByCode(selected.error_code, t)}
                  </p>
                )}

                <label className="voice-field voice-transcript-field">
                  <span>{t('转写文本', 'Transcript')}</span>
                  <textarea
                    rows={6}
                    value={transcript}
                    disabled={selected.trashed || busy !== null}
                    placeholder={t(
                      '转写完成或手工输入后，可存入收件箱。',
                      'After transcription or manual typing, save it to the inbox.',
                    )}
                    onChange={(event) => {
                      setTranscript(event.target.value);
                      setDirty(true);
                    }}
                  />
                </label>
                <p className="voice-hint">
                  {t(
                    '修订文本不会改写原始音频；重新转写前会先确认。',
                    'Editing the text never rewrites the original audio. Re-transcribing asks first.',
                  )}
                </p>

                {!dirty && selected.segments.length > 0 && (
                  <ol className="voice-segments" aria-label={t('转写分段', 'Transcript segments')}>
                    {selected.segments.map((segment, index) => (
                      <li key={`${segment.start_ms}-${index}`}>
                        <button
                          type="button"
                          onClick={() => {
                            const audio = audioRef.current;
                            if (!audio) return;
                            audio.currentTime = segment.start_ms / 1_000;
                            void audio.play().catch(() => undefined);
                          }}
                        >
                          <span className="voice-segment-time">
                            {formatDuration(segment.start_ms)}
                          </span>
                          {segment.speaker && (
                            <span className="voice-segment-speaker">{segment.speaker}</span>
                          )}
                          <span>{segment.text}</span>
                        </button>
                      </li>
                    ))}
                  </ol>
                )}

                {summary && (
                  <section className="voice-summary" aria-live="polite">
                    <header>
                      <strong>
                        {summary.mode === 'actions'
                          ? t('待办（AI 提取）', 'Actions (AI extracted)')
                          : t('摘要（AI）', 'Summary (AI)')}
                      </strong>
                      <button
                        type="button"
                        className="text-button"
                        onClick={() => setSummary(null)}
                      >
                        {t('关闭', 'Close')}
                      </button>
                    </header>
                    <p>{summary.text}</p>
                    <p className="voice-hint">
                      {t(
                        '由已配置的生成模型根据转写生成，请核对后再使用。',
                        'Generated from the transcript by your configured model. Review before use.',
                      )}
                    </p>
                  </section>
                )}

                {canPromote && (
                  <div className="voice-organize">
                    <div className="choice-field voice-field">
                      <span>{t('归入主题（可选）', 'Collection (optional)')}</span>
                      <Select
                        aria-label={t('归入主题', 'Collection')}
                        value={collectionId}
                        disabled={busy !== null}
                        onValueChange={(value) => setCollectionId(value)}
                      >
                        <option value="">{t('先留在收件箱', 'Keep in the inbox')}</option>
                        {collections.map((collection) => (
                          <option key={collection.id} value={collection.id}>
                            {collection.name}
                          </option>
                        ))}
                      </Select>
                    </div>
                    <label className="voice-field">
                      <span>
                        {t('标签（逗号分隔，最多 12 个）', 'Tags (comma separated, max 12)')}
                      </span>
                      <input
                        value={tagText}
                        maxLength={500}
                        placeholder={t('语音, 复盘', 'voice, retrospective')}
                        disabled={busy !== null}
                        onChange={(event) => setTagText(event.target.value)}
                      />
                    </label>
                  </div>
                )}
              </>
            )}
          </section>
        )}
      </div>

      {confirm === 'reload' && selected && (
        <ActionConfirmDialog
          title={t('重新载入文本？', 'Reload the text?')}
          description={t(
            '未保存的修改会被已保存的文本替换；原始音频保留。',
            'Unsaved edits will be replaced with the saved text; original audio is kept.',
          )}
          confirmLabel={t('重新载入', 'Reload')}
          onCancel={() => setConfirm(null)}
          onConfirm={() => void reloadText()}
        />
      )}
      {confirm === 'trash' && selected && (
        <ActionConfirmDialog
          title={t('移到语音回收站？', 'Move to voice trash?')}
          description={t(
            '原始音频会保留，可随时恢复。已存入收件箱的笔记不会被删除。',
            'The original audio is kept and can be restored. A note already saved to the inbox is not deleted.',
          )}
          confirmLabel={t('移到回收站', 'Move to trash')}
          tone="danger"
          onCancel={() => setConfirm(null)}
          onConfirm={() => void changeTrash(true)}
        />
      )}
      {confirm === 'restore' && selected && (
        <ActionConfirmDialog
          title={t('恢复这条录音？', 'Restore this recording?')}
          description={t('恢复后会回到待确认列表。', 'It returns to the review list.')}
          confirmLabel={t('恢复', 'Restore')}
          onCancel={() => setConfirm(null)}
          onConfirm={() => void changeTrash(false)}
        />
      )}
      {confirm === 'retranscribe' && selected && (
        <ActionConfirmDialog
          title={t('重新转写？', 'Transcribe again?')}
          description={t(
            '新结果会覆盖当前转写文本；原始音频不变。已存入收件箱的笔记不会被自动改写。',
            'The new result replaces the current transcript; the audio is unchanged. An already saved note is not updated automatically.',
          )}
          confirmLabel={t('重新转写', 'Transcribe again')}
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            setConfirm(null);
            void runTranscription();
          }}
        />
      )}
    </PageFrame>
  );
}

function statusLabel(status: VoiceCapture['status'], t: (zh: string, en: string) => string) {
  switch (status) {
    case 'transcribing':
      return t('转写中', 'Transcribing');
    case 'ready':
      return t('已就绪', 'Ready');
    case 'failed':
      return t('转写失败', 'Failed');
    default:
      return t('待转写', 'Needs transcript');
  }
}

function recorderErrorText(error: unknown, t: (zh: string, en: string) => string) {
  const code = error instanceof RecorderError ? error.code : 'recording_failed';
  switch (code) {
    case 'microphone_denied':
      return t(
        '麦克风权限被拒绝。请在 Windows 设置 → 隐私 → 麦克风 中允许桌面应用访问，或导入音频文件。',
        'Microphone access was denied. Allow desktop apps in Windows Settings → Privacy → Microphone, or import an audio file.',
      );
    case 'microphone_missing':
      return t(
        '未检测到麦克风。请连接设备，或导入手机/录音笔的音频。',
        'No microphone was found. Connect one, or import audio from your phone or recorder.',
      );
    case 'microphone_busy':
      return t(
        '麦克风正被其他程序占用。关闭占用程序后重试。',
        'The microphone is used by another app. Close it and try again.',
      );
    case 'recorder_unsupported':
      return t(
        '当前 WebView 不支持录音。请导入音频文件。',
        'This WebView cannot record audio. Import an audio file instead.',
      );
    case 'system_audio_denied':
      return t(
        '会议录音需要同时授权麦克风和屏幕/系统声音；本次被拒绝。',
        'Meeting recording needs both microphone and screen/system-audio permission; this request was denied.',
      );
    case 'system_audio_unavailable':
      return t(
        '当前环境无法采集系统声音。请改用导入音频，或检查 Windows 声音设置。',
        'System audio capture is unavailable here. Import an audio file or check Windows sound settings.',
      );
    default:
      return t(
        '录音失败。请重试或导入音频文件。',
        'Recording failed. Try again or import an audio file.',
      );
  }
}

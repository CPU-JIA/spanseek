import { invoke, isTauri } from '@tauri-apps/api/core';
import type {
  AsrSettingsState,
  AsrTestResult,
  CreateVoiceCapture,
  SaveVoiceCaptureNote,
  UpdateAsrSettings,
  UpdateVoiceCapture,
  VoiceCapture,
  VoiceCapturePage,
  VoiceBrowseRequest,
  VoiceBrowsePage,
  VoiceCaptureTrashRequest,
  VoiceSummary,
  VoiceSummaryMode,
  VoiceSyncImport,
  VoiceSyncTask,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message, ServiceError } from '../api';

export function listVoiceCaptures(
  identity: WorkspaceIdentity,
  trashed: boolean,
): Promise<VoiceCapturePage> {
  return call({ kind: 'voice_captures', payload: { trashed } }, 'voice_captures', identity);
}

export function browseVoiceCaptures(
  identity: WorkspaceIdentity,
  request: VoiceBrowseRequest,
): Promise<VoiceBrowsePage> {
  return call({ kind: 'voice_browse', payload: request }, 'voice_browse', identity);
}
export function getVoiceCapture(
  identity: WorkspaceIdentity,
  captureId: string,
): Promise<VoiceCapture> {
  return call(
    { kind: 'voice_capture_get', payload: { capture_id: captureId } },
    'voice_capture',
    identity,
  );
}

export function createVoiceCapture(
  identity: WorkspaceIdentity,
  request: CreateVoiceCapture,
  audioBase64: string,
): Promise<VoiceCapture> {
  return call(
    {
      kind: 'voice_capture_create',
      payload: { command_id: crypto.randomUUID(), request, audio_base64: audioBase64 },
    },
    'voice_capture',
    identity,
  );
}

export function updateVoiceCapture(
  identity: WorkspaceIdentity,
  request: UpdateVoiceCapture,
): Promise<VoiceCapture> {
  return call({ kind: 'voice_capture_update', payload: { request } }, 'voice_capture', identity);
}

export function trashVoiceCapture(
  identity: WorkspaceIdentity,
  request: VoiceCaptureTrashRequest,
): Promise<VoiceCapture> {
  return call({ kind: 'voice_capture_trash', payload: { request } }, 'voice_capture', identity);
}

export function saveVoiceCaptureNote(
  identity: WorkspaceIdentity,
  request: SaveVoiceCaptureNote,
): Promise<VoiceCapture> {
  return call(
    {
      kind: 'voice_capture_save_note',
      payload: { command_id: crypto.randomUUID(), request },
    },
    'voice_capture',
    identity,
  );
}

export function transcribeVoiceCapture(
  identity: WorkspaceIdentity,
  captureId: string,
  baseRowVersion: number,
): Promise<VoiceCapture> {
  return call(
    {
      kind: 'voice_capture_transcribe',
      payload: { capture_id: captureId, base_row_version: baseRowVersion },
    },
    'voice_capture',
    identity,
  );
}

export function getAsrSettings(identity: WorkspaceIdentity): Promise<AsrSettingsState> {
  return call({ kind: 'asr_settings_get' }, 'asr_settings', identity);
}

export function saveAsrSettings(
  identity: WorkspaceIdentity,
  request: UpdateAsrSettings,
  credential: string | null,
  clearCredential: boolean,
): Promise<AsrSettingsState> {
  return call(
    {
      kind: 'asr_settings_save',
      payload: {
        request,
        credential: credential && credential.length > 0 ? credential : null,
        clear_credential: clearCredential,
      },
    },
    'asr_settings',
    identity,
  );
}

export function testAsrSettings(identity: WorkspaceIdentity): Promise<AsrTestResult> {
  return call({ kind: 'asr_settings_test' }, 'asr_test_result', identity, 150_000);
}

export function summarizeVoiceCapture(
  identity: WorkspaceIdentity,
  captureId: string,
  mode: VoiceSummaryMode,
): Promise<VoiceSummary> {
  return call(
    { kind: 'voice_capture_summarize', payload: { request: { capture_id: captureId, mode } } },
    'voice_summary',
    identity,
    150_000,
  );
}

export function importVoiceSync(identity: WorkspaceIdentity): Promise<VoiceSyncImport> {
  return call({ kind: 'voice_sync_import' }, 'voice_sync_import', identity, 150_000);
}

export function listVoiceSyncTasks(identity: WorkspaceIdentity): Promise<VoiceSyncTask[]> {
  return call({ kind: 'voice_sync_tasks' }, 'voice_sync_tasks', identity);
}
export async function loadVoiceAudio(
  identity: WorkspaceIdentity,
  captureId: string,
): Promise<ArrayBuffer> {
  if (!isTauri()) throw new Error('请通过 spanseek 桌面应用打开。');
  return invoke<ArrayBuffer>('voice_capture_audio', { identity, captureId });
}

export function saveVoiceAudio(identity: WorkspaceIdentity, captureId: string): Promise<boolean> {
  return invoke<boolean>('voice_capture_export', { identity, captureId });
}

/** Voice failures carry a stable reason so English never falls back to Chinese. */
export function voiceFailureReason(error: unknown): string | null {
  if (!(error instanceof ServiceError)) return null;
  const details = error.details.details;
  if (details?.kind === 'voice_failure') return details.reason;
  if (
    details?.kind === 'inference_failure' &&
    [
      'remote_consent_required',
      'model_profile_missing',
      'model_key_missing',
      'model_incomplete',
    ].includes(details.reason)
  ) {
    return details.reason;
  }
  return null;
}

export function voiceErrorMessage(error: unknown): string {
  const reason = voiceFailureReason(error);
  if (reason) return reason;
  return message(error);
}

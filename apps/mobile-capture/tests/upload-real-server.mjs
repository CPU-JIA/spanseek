import { uploadVoiceCapture } from '../sync.js';

const fixture = JSON.parse(process.env.VOICE_SYNC_FIXTURE);
const profile = {
  ...fixture.profile,
  edPrivateJwk: fixture.edPrivateJwk,
  keys: { '1': fixture.workspaceKeyBase64 },
};
const audio = Uint8Array.from([
  0x1a, 0x45, 0xdf, 0xa3, ...new TextEncoder().encode('real-server-mobile-upload'),
]);
await uploadVoiceCapture(profile, {
  id: fixture.captureId,
  durationMs: 5000,
  blob: new Blob([audio], { type: 'audio/webm' }),
});
console.log('uploaded');

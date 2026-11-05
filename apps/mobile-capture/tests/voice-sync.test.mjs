import test from 'node:test';
import assert from 'node:assert/strict';
import { createIdentity, uploadVoiceCapture } from '../sync.js';

function canonical(parts) {
  const chunks = [];
  let length = 0;
  for (const part of parts) {
    const bytes = typeof part === 'string' ? new TextEncoder().encode(part) : part;
    const header = new Uint8Array(8);
    new DataView(header.buffer).setBigUint64(0, BigInt(bytes.length));
    chunks.push(header, bytes);
    length += header.length + bytes.length;
  }
  const out = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

function i64(value) {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, BigInt(value));
  return bytes;
}

function fromBase64(value) {
  return Uint8Array.from(atob(value), (char) => char.charCodeAt(0));
}

function toHex(bytes) {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

test('uploadVoiceCapture encrypts, chunks, finalizes and writes a voice marker', async () => {
  const identity = await createIdentity('test phone');
  const rawKey = crypto.getRandomValues(new Uint8Array(32));
  const workspaceId = 'workspace-voice-test';
  const epoch = 1;
  const key = await crypto.subtle.importKey('raw', rawKey, 'AES-GCM', false, ['encrypt', 'decrypt']);
  const profile = {
    ...identity,
    workspaceId,
    epoch,
    deviceId: crypto.randomUUID(),
    endpoint: 'http://mock-sync',
    keys: { '1': btoa(String.fromCharCode(...rawKey)) },
  };

  const calls = [];
  globalThis.fetch = async (url, init) => {
    const body = JSON.parse(init.body);
    calls.push({ path: new URL(url).pathname, body });
    if (url.endsWith('/v1/keys/epoch')) {
      return new Response(JSON.stringify(1), { status: 200 });
    }
    if (url.endsWith('/v1/blobs/begin')) {
      return new Response(
        JSON.stringify({
          already_present: false,
          status: {
            blob_hash: body.blob_hash,
            size: body.size,
            chunk_size: body.chunk_size,
            chunk_count: Math.ceil(body.size / body.chunk_size),
            complete: false,
            received_chunks: [],
            missing_chunks: [],
          },
        }),
        { status: 200 },
      );
    }
    if (url.endsWith('/v1/ops')) {
      return new Response(JSON.stringify({ status: 'applied', version: 1 }), { status: 200 });
    }
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };

  const audio = new Uint8Array([
    0x1a, 0x45, 0xdf, 0xa3, ...new TextEncoder().encode('voice-sync-mobile-test'),
  ]);
  const plaintextHash = toHex(new Uint8Array(await crypto.subtle.digest('SHA-256', audio)));
  const recording = {
    id: crypto.randomUUID(),
    durationMs: 4321,
    blob: new Blob([audio], { type: 'audio/webm' }),
  };

  await uploadVoiceCapture(profile, recording);

  const paths = calls.map((call) => call.path);
  assert.deepEqual(paths, [
    '/v1/keys/epoch',
    '/v1/blobs/begin',
    '/v1/blobs/chunk',
    '/v1/blobs/finalize',
    '/v1/ops',
  ]);

  const begin = calls.find((call) => call.path === '/v1/blobs/begin').body;
  const chunk = calls.find((call) => call.path === '/v1/blobs/chunk').body;
  const ops = calls.find((call) => call.path === '/v1/ops').body;
  assert.equal(begin.chunk_size, 64 * 1024);
  assert.equal(begin.size, fromBase64(chunk.data).length);

  const sealed = fromBase64(chunk.data);
  const aad = canonical([
    'spanseek-sync-voice-blob-aad-v1',
    workspaceId,
    plaintextHash,
    i64(epoch),
  ]);
  const plain = new Uint8Array(
    await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv: sealed.slice(0, 12), additionalData: aad },
      key,
      sealed.slice(12),
    ),
  );
  assert.deepEqual(plain, audio);

  const noteAad = canonical([
    'spanseek-sync-object-aad-v1',
    workspaceId,
    recording.id,
    i64(epoch),
  ]);
  const noteSealed = fromBase64(ops.op.ciphertext);
  const note = JSON.parse(
    new TextDecoder().decode(
      await crypto.subtle.decrypt(
        { name: 'AES-GCM', iv: noteSealed.slice(0, 12), additionalData: noteAad },
        key,
        noteSealed.slice(12),
      ),
    ),
  );
  const text = note.document.content
    .flatMap((block) => block.content ?? [])
    .map((node) => node.text ?? '')
    .join('');
  assert.match(text, /<!-- spanseek-voice:v1:/);
  assert.match(text, new RegExp(begin.blob_hash));
  assert.match(text, new RegExp(plaintextHash));
  assert.match(text, /4321/);
  assert.match(text, /audio\/webm/);
});

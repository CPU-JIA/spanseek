// Protocol-v1 client for the mobile capture PWA.
//
// The PWA holds an Ed25519 device signing key and an X25519 wrapping key in
// IndexedDB. The text encoder, SHA-256, HKDF and AES-GCM are all WebCrypto.
// The sync authority only sees ciphertext and signatures.

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });
const ACCESS_PREFIX = 'spanseek-sync-access-v1';
const CONFIG_KINDS = new Set(['view', 'canvas', 'workspace_metadata', 'tag', 'tag_assignments']);
const DOCUMENT_SCHEMAS = new Set([1, 2, 3]);
const DOCUMENT_NODES = new Set([
  'paragraph',
  'heading',
  'text',
  'hardBreak',
  'bulletList',
  'orderedList',
  'listItem',
  'taskList',
  'taskItem',
  'blockquote',
  'codeBlock',
  'horizontalRule',
  'image',
  'noteReference',
  'table',
  'tableRow',
  'tableCell',
  'tableHeader',
  'callout',
  'inlineMath',
  'blockMath',
  'wikiLink',
  'wikiEmbed',
  'footnoteReference',
  'footnoteDefinition',
  'comment',
  'commentBlock',
  'frontmatter',
]);
const SCHEMA_THREE_NODES = new Set([
  'callout',
  'inlineMath',
  'blockMath',
  'wikiLink',
  'wikiEmbed',
  'footnoteReference',
  'footnoteDefinition',
  'comment',
  'commentBlock',
  'frontmatter',
]);
const DOCUMENT_MARKS = new Set([
  'bold',
  'italic',
  'strike',
  'code',
  'link',
  'underline',
  'superscript',
  'subscript',
  'highlight',
  'textStyle',
]);
const SCHEMA_THREE_MARKS = new Set([
  'underline',
  'superscript',
  'subscript',
  'highlight',
  'textStyle',
]);
const INLINE_NODES = new Set([
  'text',
  'hardBreak',
  'noteReference',
  'inlineMath',
  'wikiLink',
  'footnoteReference',
  'comment',
]);

export class MobileSyncError extends Error {
  constructor(code, message, details) {
    super(message || code);
    this.name = 'MobileSyncError';
    this.code = code;
    this.details = details;
  }
}

function endpointBase(value) {
  const url = new URL(value);
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) {
    throw new MobileSyncError('invalid_endpoint', '同步服务器地址必须是 http(s) 地址。');
  }
  return url.toString().replace(/\/+$/, '');
}

function concatBytes(chunks) {
  const length = chunks.reduce((total, chunk) => total + chunk.length, 0);
  const out = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

function canonical(parts) {
  const chunks = [];
  for (const part of parts) {
    const bytes = typeof part === 'string' ? encoder.encode(part) : part;
    const length = new Uint8Array(8);
    new DataView(length.buffer).setBigUint64(0, BigInt(bytes.length));
    chunks.push(length, bytes);
  }
  return concatBytes(chunks);
}

function i64(value) {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigInt64(0, BigInt(value));
  return bytes;
}

function u64(value) {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, BigInt(value));
  return bytes;
}

function toHex(bytes) {
  return Array.from(bytes, (value) => value.toString(16).padStart(2, '0')).join('');
}

function fromHex(text) {
  if (typeof text !== 'string' || text.length % 2 !== 0 || !/^[0-9a-f]*$/i.test(text)) {
    throw new MobileSyncError('invalid_hex', '同步数据不是有效的十六进制。');
  }
  const out = new Uint8Array(text.length / 2);
  for (let index = 0; index < out.length; index += 1) {
    out[index] = Number.parseInt(text.slice(index * 2, index * 2 + 2), 16);
  }
  return out;
}

function toBase64(bytes) {
  let binary = '';
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}

function fromBase64(text) {
  if (typeof text !== 'string') throw new MobileSyncError('invalid_base64', '同步数据缺少密文。');
  let binary;
  try {
    binary = atob(text);
  } catch {
    throw new MobileSyncError(
      'invalid_base64',
      '同步数据编码无效，原操作已保留。请重新同步或核对密钥授权。',
    );
  }
  const out = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) out[index] = binary.charCodeAt(index);
  return out;
}

function bytesValue(value) {
  if (Array.isArray(value)) return Uint8Array.from(value);
  if (typeof value === 'string') return fromBase64(value);
  throw new MobileSyncError('invalid_bytes', '同步服务器返回了无法识别的字节字段。');
}

function base64UrlBytes(value) {
  const padded = value
    .replace(/-/g, '+')
    .replace(/_/g, '/')
    .padEnd(Math.ceil(value.length / 4) * 4, '=');
  return fromBase64(padded);
}

async function sha256(bytes) {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
}

async function canonicalHash(parts) {
  return sha256(canonical(parts));
}

async function importEdPrivate(jwk) {
  return crypto.subtle.importKey('jwk', jwk, { name: 'Ed25519' }, false, ['sign']);
}

async function importXPrivate(jwk) {
  return crypto.subtle.importKey('jwk', jwk, { name: 'X25519' }, false, ['deriveBits']);
}

function normalizedCode(code) {
  return String(code).replace(/[\s-]/g, '').toUpperCase();
}

export async function createIdentity(displayName) {
  const ed = await crypto.subtle.generateKey({ name: 'Ed25519' }, true, ['sign', 'verify']);
  const x = await crypto.subtle.generateKey({ name: 'X25519' }, true, ['deriveBits']);
  const edPrivateJwk = await crypto.subtle.exportKey('jwk', ed.privateKey);
  const edPublicJwk = await crypto.subtle.exportKey('jwk', ed.publicKey);
  const xPrivateJwk = await crypto.subtle.exportKey('jwk', x.privateKey);
  const xPublicJwk = await crypto.subtle.exportKey('jwk', x.publicKey);
  return {
    displayName,
    edPrivateJwk,
    edPublicHex: toHex(base64UrlBytes(edPublicJwk.x)),
    xPrivateJwk,
    xPublicHex: toHex(base64UrlBytes(xPublicJwk.x)),
    keys: {},
    cursor: 0,
    epoch: 0,
    workspaceId: '',
    endpoint: '',
    deviceId: '',
  };
}

async function signedRequest(profile, access, requestHash) {
  if (!profile.deviceId) throw new MobileSyncError('not_paired', '先加入同步再执行此操作。');
  const nonce = crypto.randomUUID().replaceAll('-', '');
  const issuedAt = Math.floor(Date.now() / 1000);
  const message = canonical([
    ACCESS_PREFIX,
    profile.deviceId,
    access,
    nonce,
    i64(issuedAt),
    requestHash,
  ]);
  const key = await importEdPrivate(profile.edPrivateJwk);
  const signature = new Uint8Array(await crypto.subtle.sign({ name: 'Ed25519' }, key, message));
  return {
    device_id: profile.deviceId,
    access,
    nonce,
    issued_at: issuedAt,
    request_hash: toHex(requestHash),
    signature: toBase64(signature),
  };
}

async function postJson(url, body) {
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  let payload = null;
  try {
    payload = text ? JSON.parse(text) : null;
  } catch {
    payload = null;
  }
  if (!response.ok) {
    throw new MobileSyncError(
      payload?.code || 'http_error',
      payload?.message || `同步服务器返回 HTTP ${response.status}。`,
      { status: response.status, body: payload },
    );
  }
  return payload;
}

async function signedPost(profile, path, access, requestHash, extra = {}) {
  const request = await signedRequest(profile, access, requestHash);
  return postJson(`${profile.endpoint}${path}`, { request, ...extra });
}

export async function pairDevice(profile, { endpoint, displayName, inviteId, code }) {
  const next = profile?.edPrivateJwk
    ? { ...profile, displayName: displayName || profile.displayName }
    : await createIdentity(displayName || '手机');
  next.endpoint = endpointBase(endpoint);
  const proofMessage = canonical([
    'spanseek-sync-pair-v1',
    inviteId,
    normalizedCode(code),
    next.edPublicHex,
  ]);
  const key = await importEdPrivate(next.edPrivateJwk);
  const signature = new Uint8Array(
    await crypto.subtle.sign({ name: 'Ed25519' }, key, proofMessage),
  );
  const device = await postJson(`${next.endpoint}/v1/pairing/complete`, {
    invite_id: inviteId,
    code: normalizedCode(code),
    public_key: next.edPublicHex,
    display_name: next.displayName,
    proof: toBase64(signature),
  });
  next.deviceId = device.device_id;
  const registerHash = await canonicalHash(['spanseek-sync-device-encryption-v1', next.xPublicHex]);
  await signedPost(next, '/v1/keys/device', 'write', registerHash, {
    encryption_public_key: next.xPublicHex,
  });
  return next;
}

function toBase64Url(bytes) {
  return toBase64(bytes).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

async function openWorkspaceKey(profile, envelope, epoch) {
  const ephemeral = bytesValue(envelope.ephemeral_public_key);
  const nonce = bytesValue(envelope.nonce);
  const ciphertext = bytesValue(envelope.ciphertext);
  if (ephemeral.length !== 32 || nonce.length !== 12) {
    throw new MobileSyncError('invalid_key_envelope', '同步密钥信封格式不正确。');
  }
  const privateKey = await importXPrivate(profile.xPrivateJwk);
  const rawPublic = fromHex(profile.xPublicHex);
  const sender = await crypto.subtle.importKey(
    'jwk',
    { kty: 'OKP', crv: 'X25519', x: toBase64Url(ephemeral), ext: true, key_ops: [] },
    { name: 'X25519' },
    false,
    [],
  );
  const shared = new Uint8Array(
    await crypto.subtle.deriveBits({ name: 'X25519', public: sender }, privateKey, 256),
  );
  const salt = concatBytes([ephemeral, rawPublic]);
  const hkdf = await crypto.subtle.importKey('raw', shared, 'HKDF', false, ['deriveBits']);
  const wrappingKey = new Uint8Array(
    await crypto.subtle.deriveBits(
      {
        name: 'HKDF',
        hash: 'SHA-256',
        salt,
        info: encoder.encode('spanseek-sync-wsk-wrap-v1'),
      },
      hkdf,
      256,
    ),
  );
  const aes = await crypto.subtle.importKey('raw', wrappingKey, 'AES-GCM', false, ['decrypt']);
  const aad = canonical([
    'spanseek-sync-wsk-aad-v1',
    profile.workspaceId,
    i64(epoch),
    profile.deviceId,
  ]);
  try {
    return new Uint8Array(
      await crypto.subtle.decrypt(
        { name: 'AES-GCM', iv: nonce, additionalData: aad },
        aes,
        ciphertext,
      ),
    );
  } catch {
    throw new MobileSyncError('key_unwrap_failed', '无法解封同步密钥，可能授权给了另一台设备。');
  }
}

export async function attachWorkspace(profile) {
  if (!profile.workspaceId) throw new MobileSyncError('workspace_missing', '先绑定知识库 ID。');
  const currentHash = await canonicalHash([
    'spanseek-sync-current-key-epoch-v1',
    profile.workspaceId,
  ]);
  const current = await signedPost(profile, '/v1/keys/epoch', 'read', currentHash, {
    workspace_id: profile.workspaceId,
  });
  if (!Number.isInteger(current) || current < 1) {
    throw new MobileSyncError('key_not_granted', '另一台设备还没有授权同步密钥。');
  }
  const next = { ...profile, keys: { ...(profile.keys || {}) }, epoch: current };
  for (let epoch = 1; epoch <= current; epoch += 1) {
    if (next.keys[String(epoch)]) continue;
    const envelopeHash = await canonicalHash([
      'spanseek-sync-key-envelope-v1',
      next.workspaceId,
      i64(epoch),
    ]);
    let envelope;
    try {
      envelope = await signedPost(profile, '/v1/keys/envelope', 'read', envelopeHash, {
        workspace_id: next.workspaceId,
        epoch,
      });
    } catch (error) {
      if (error instanceof MobileSyncError && error.code === 'key_not_found') {
        throw new MobileSyncError('key_not_granted', `第 ${epoch} 个同步密钥尚未授权给本机。`);
      }
      throw error;
    }
    const key = await openWorkspaceKey(next, envelope, epoch);
    next.keys[String(epoch)] = toBase64(key);
  }
  return next;
}

async function objectKey(profile, epoch) {
  const encoded = profile.keys?.[String(epoch)];
  if (!encoded) throw new MobileSyncError('missing_workspace_key', `缺少第 ${epoch} 个同步密钥。`);
  return crypto.subtle.importKey('raw', fromBase64(encoded), 'AES-GCM', false, [
    'decrypt',
    'encrypt',
  ]);
}

function objectAad(workspaceId, objectId, epoch) {
  return canonical(['spanseek-sync-object-aad-v1', workspaceId, objectId, i64(epoch)]);
}

function documentFromText(body) {
  const paragraphs = String(body)
    .split(/\n+/)
    .filter((line, index, all) => line.length > 0 || all.length === 1)
    .map((text) => ({
      type: 'paragraph',
      attrs: { id: crypto.randomUUID() },
      content: text ? [{ type: 'text', text }] : [],
    }));
  return { type: 'doc', content: paragraphs };
}

function textFromDocument(document) {
  const blocks = [];
  const visit = (node, parts) => {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'text' && typeof node.text === 'string') parts.push(node.text);
    if (Array.isArray(node.content)) {
      for (const child of node.content) visit(child, parts);
    }
  };
  for (const block of document?.content || []) {
    const parts = [];
    visit(block, parts);
    blocks.push(parts.join(''));
  }
  return blocks.join('\n\n');
}

function record(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function invalidNote(message) {
  return new MobileSyncError('invalid_note_payload', message);
}

function validTimestamp(value) {
  if (typeof value !== 'string') return false;
  const match =
    /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:Z|[+-](\d{2}):(\d{2}))$/.exec(
      value,
    );
  if (!match || !Number.isFinite(Date.parse(value))) return false;
  const [, year, month, day, hour, minute, second, zoneHour = '0', zoneMinute = '0'] = match;
  const days = [
    31,
    Number(year) % 4 === 0 && (Number(year) % 100 !== 0 || Number(year) % 400 === 0) ? 29 : 28,
    31,
    30,
    31,
    30,
    31,
    31,
    30,
    31,
    30,
    31,
  ];
  return (
    Number(month) >= 1 &&
    Number(month) <= 12 &&
    Number(day) >= 1 &&
    Number(day) <= days[Number(month) - 1] &&
    Number(hour) <= 23 &&
    Number(minute) <= 59 &&
    Number(second) <= 59 &&
    Number(zoneHour) <= 23 &&
    Number(zoneMinute) <= 59
  );
}

function validateNotePayload(payload) {
  if (typeof payload.title !== 'string' || !validTimestamp(payload.updated_at)) {
    throw invalidNote('同步笔记缺少有效标题或更新时间，未创建本机内容。请检查来源后重试。');
  }
  if (!DOCUMENT_SCHEMAS.has(payload.document_schema)) {
    throw new MobileSyncError(
      'unsupported_document_schema',
      '此笔记正文版本暂不受支持，请更新客户端后重试。',
    );
  }
  const document = payload.document;
  if (
    !record(document) ||
    document.type !== 'doc' ||
    !Array.isArray(document.content) ||
    !document.content.length ||
    document.content.some(
      (node) =>
        record(node) &&
        (INLINE_NODES.has(node.type) ||
          ['listItem', 'taskItem', 'tableRow', 'tableCell', 'tableHeader'].includes(node.type)),
    )
  ) {
    throw invalidNote('同步笔记正文结构无效，原操作已保留待重试。');
  }
  // Validate the read projection's structure rather than guessing through malformed
  // content. Full editor attribute/business validation remains in the Rust core.
  const stack = document.content.map((node) => ({ node, depth: 1 }));
  let count = 0;
  while (stack.length) {
    const { node, depth } = stack.pop();
    if (++count > 50_000 || depth > 32)
      throw invalidNote('同步笔记的节点数量或嵌套深度超出支持范围。');
    if (!record(node) || typeof node.type !== 'string')
      throw invalidNote('同步笔记包含无法读取的正文节点。');
    if (
      !DOCUMENT_NODES.has(node.type) ||
      (payload.document_schema < 3 && SCHEMA_THREE_NODES.has(node.type))
    ) {
      throw new MobileSyncError(
        'unsupported_document_node',
        '此笔记包含暂不支持的正文类型，请更新客户端后重试。',
      );
    }
    if (
      (node.type === 'text' && typeof node.text !== 'string') ||
      (node.type !== 'text' && node.text !== undefined && node.text !== null) ||
      (node.type === 'text' && node.content?.length) ||
      (node.text !== undefined && node.text !== null && typeof node.text !== 'string') ||
      (node.content !== undefined && !Array.isArray(node.content)) ||
      (node.attrs !== undefined && !record(node.attrs)) ||
      (node.marks !== undefined && !Array.isArray(node.marks))
    ) {
      throw invalidNote('同步笔记的文字、属性或子节点格式无效，未生成替代正文。');
    }
    for (const mark of node.marks ?? []) {
      if (
        !record(mark) ||
        !DOCUMENT_MARKS.has(mark.type) ||
        (payload.document_schema < 3 && SCHEMA_THREE_MARKS.has(mark.type)) ||
        (mark.attrs !== undefined && !record(mark.attrs))
      ) {
        throw new MobileSyncError(
          'unsupported_document_mark',
          '此笔记包含无法识别的文字格式，请更新客户端后重试。',
        );
      }
    }
    for (const child of node.content ?? []) stack.push({ node: child, depth: depth + 1 });
  }
  if (
    payload.document_schema === 1 &&
    document.content.some(
      (block) =>
        block.type !== 'paragraph' ||
        (block.content ?? []).some((child) => child.type !== 'text' || child.marks?.length),
    )
  ) {
    throw invalidNote('旧版笔记正文不符合其版本格式，未转换或覆盖内容。');
  }
}

async function decryptPayload(profile, op) {
  const ciphertext = fromBase64(op.ciphertext);
  if (ciphertext.length < 28)
    throw new MobileSyncError('invalid_ciphertext', '同步密文不完整，原操作已保留待重试。');
  const nonce = ciphertext.slice(0, 12);
  const key = await objectKey(profile, op.key_epoch);
  let plaintext;
  try {
    plaintext = new Uint8Array(
      await crypto.subtle.decrypt(
        {
          name: 'AES-GCM',
          iv: nonce,
          additionalData: objectAad(profile.workspaceId, op.object_id, op.key_epoch),
        },
        key,
        ciphertext.slice(12),
      ),
    );
  } catch {
    throw new MobileSyncError('decrypt_failed', '无法解密这条同步内容，请核对密钥授权后重试。');
  }
  let payload;
  try {
    payload = JSON.parse(decoder.decode(plaintext));
  } catch {
    throw new MobileSyncError(
      'invalid_sync_payload',
      '同步内容不是有效的 UTF-8 JSON，原操作已保留待重试。',
    );
  }
  if (!record(payload))
    throw new MobileSyncError('invalid_sync_payload', '同步内容不是有效的对象载荷。');
  if (Object.hasOwn(payload, 'sync_kind')) {
    // These objects belong to desktop configuration. They are deliberately not
    // mobile captures, even if a configuration happens to contain note-like keys.
    if (CONFIG_KINDS.has(payload.sync_kind)) return null;
    throw new MobileSyncError(
      'unsupported_sync_payload',
      '此同步内容类型暂不受支持，请更新客户端后重试。',
    );
  }
  validateNotePayload(payload);
  return {
    objectId: op.object_id,
    title: payload.title,
    body: textFromDocument(payload.document),
    updatedAt: payload.updated_at,
    version: op.resulting_version,
  };
}

function validatePullBatch(batch, cursor, workspaceId) {
  if (
    !record(batch) ||
    !Array.isArray(batch.ops) ||
    !Number.isSafeInteger(batch.next_cursor) ||
    batch.next_cursor < cursor ||
    typeof batch.has_more !== 'boolean'
  ) {
    throw new MobileSyncError('invalid_pull_batch', '同步服务器返回的批次或位置无效，请重试。');
  }
  let previous = cursor;
  for (const op of batch.ops) {
    if (
      !record(op) ||
      !Number.isSafeInteger(op.server_seq) ||
      op.server_seq <= previous ||
      op.workspace_id !== workspaceId ||
      typeof op.object_id !== 'string' ||
      !op.object_id ||
      !Number.isSafeInteger(op.resulting_version) ||
      op.resulting_version < 1 ||
      !Number.isSafeInteger(op.key_epoch) ||
      op.key_epoch < 1 ||
      !['put', 'restore', 'delete', 'purge'].includes(op.kind)
    ) {
      throw new MobileSyncError(
        'invalid_pull_batch',
        '同步操作的顺序、对象或知识库信息无效，未跳过原位置。',
      );
    }
    previous = op.server_seq;
  }
  if (batch.next_cursor !== previous || (batch.has_more && !batch.ops.length)) {
    throw new MobileSyncError(
      'invalid_pull_batch',
      '同步批次的末尾位置与实际操作不一致，未推进位置。',
    );
  }
}

async function encryptNote(profile, capture, epoch) {
  const payload = {
    title: capture.title || '未命名捕捉',
    document_schema: 3,
    document: documentFromText(capture.body || ''),
    updated_at: capture.updatedAt || new Date().toISOString(),
  };
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const key = await objectKey(profile, epoch);
  const ciphertext = new Uint8Array(
    await crypto.subtle.encrypt(
      {
        name: 'AES-GCM',
        iv: nonce,
        additionalData: objectAad(profile.workspaceId, capture.objectId, epoch),
      },
      key,
      encoder.encode(JSON.stringify(payload)),
    ),
  );
  return new Uint8Array([...nonce, ...ciphertext]);
}

async function pushCapture(profile, capture) {
  const epoch = profile.epoch;
  const ciphertext = await encryptNote(profile, capture, epoch);
  const baseVersion = Number.isInteger(capture.baseVersion) ? capture.baseVersion : 0;
  const clientTime = new Date().toISOString();
  const opIdSource = await canonicalHash([
    'spanseek-mobile-op-v1',
    profile.workspaceId,
    capture.objectId,
    i64(baseVersion),
    i64(epoch),
    ciphertext,
  ]);
  const op = {
    op_id: `spanseek-op-${toHex(opIdSource)}`,
    workspace_id: profile.workspaceId,
    object_id: capture.objectId,
    kind: 'put',
    base_version: baseVersion,
    key_epoch: epoch,
    ciphertext: toBase64(ciphertext),
    client_time: clientTime,
  };
  const requestHash = await canonicalHash([
    'spanseek-sync-op-v1',
    op.op_id,
    op.workspace_id,
    op.object_id,
    op.kind,
    i64(op.base_version),
    i64(op.key_epoch),
    ciphertext,
    op.client_time,
  ]);
  return signedPost(profile, '/v1/ops', 'write', requestHash, { op });
}

export async function syncMobile(profile, captures, hooks = {}) {
  let next = await attachWorkspace(profile);
  let cursor = next.cursor ?? 0;
  if (!Number.isSafeInteger(cursor) || cursor < 0) {
    throw new MobileSyncError('invalid_sync_cursor', '本机同步位置无效，请核对同步绑定后重试。');
  }
  const remoteChanges = [];
  const conflicts = [];
  let receiveBlocked = false;
  let hasMore = false;
  receive: for (let page = 0; page < 20; page += 1) {
    const hash = await canonicalHash([
      'spanseek-sync-pull-v1',
      next.workspaceId,
      i64(cursor),
      u64(200),
    ]);
    const batch = await signedPost(next, '/v1/pull', 'read', hash, {
      workspace_id: next.workspaceId,
      after_seq: cursor,
      limit: 200,
    });
    validatePullBatch(batch, cursor, next.workspaceId);
    hasMore = batch.has_more;
    for (const op of batch.ops) {
      try {
        if (op.kind === 'delete' || op.kind === 'purge') {
          if (
            !hooks.onDelete &&
            captures.some(
              (capture) =>
                capture.workspaceId === next.workspaceId &&
                (capture.objectId || capture.id) === op.object_id,
            )
          ) {
            throw new MobileSyncError(
              'remote_handler_missing',
              '本机尚未准备好保存同步删除，请稍后重试。',
            );
          }
          await hooks.onDelete?.({
            objectId: op.object_id,
            version: op.resulting_version,
            purged: op.purged,
            deletedAt: op.deleted_at,
          });
        } else {
          const note = await decryptPayload(next, op);
          if (note) {
            if (!hooks.onRemote)
              throw new MobileSyncError(
                'remote_handler_missing',
                '本机尚未准备好保存同步内容，请稍后重试。',
              );
            const accepted = await hooks.onRemote(note);
            // A false return is the application's existing keep-local/conflict
            // decision, not an I/O exception. Do not retry it as a failed write.
            if (accepted !== false) remoteChanges.push(note.objectId);
          }
        }
        cursor = op.server_seq;
      } catch (error) {
        const conflict = {
          objectId: op.object_id,
          code: error instanceof MobileSyncError ? error.code : 'remote_apply_failed',
          message:
            error instanceof MobileSyncError
              ? error.message
              : '未能保存这条同步内容，已保留同步位置。请重试。',
        };
        conflicts.push(conflict);
        await hooks.onConflict?.(conflict);
        receiveBlocked = true;
        // Later operations must not move the cursor past an unknown payload,
        // failed decryption, or a write the application could not persist.
        break receive;
      }
    }
    if (!batch.has_more) break;
  }
  for (const capture of captures) {
    if (capture.workspaceId !== next.workspaceId) continue;
    if ((capture.baseVersion || 0) > 0 || capture.trashed) continue;
    const result = await pushCapture(next, capture);
    if (result.status === 'applied') {
      await hooks.onVersion?.(capture.objectId, result.version);
    } else if (result.status === 'conflict') {
      const conflict = {
        objectId: capture.objectId,
        code: 'version_mismatch',
        message: '桌面端已修改这条内容，先拉取或人工比较后再推送。',
        remoteVersion: result.current_version,
        remoteTombstone: result.current_tombstone,
      };
      conflicts.push(conflict);
      await hooks.onConflict?.(conflict);
    }
  }
  next = { ...next, cursor };
  return { profile: next, remoteChanges, conflicts, receiveBlocked, hasMore };
}

function u32(value) {
  const bytes = new Uint8Array(4);
  new DataView(bytes.buffer).setUint32(0, value);
  return bytes;
}

async function encryptVoiceBlob(profile, blob) {
  const plaintext = new Uint8Array(await blob.arrayBuffer());
  const plaintextHash = toHex(await sha256(plaintext));
  const key = await objectKey(profile, profile.epoch);
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const aad = canonical([
    'spanseek-sync-voice-blob-aad-v1',
    profile.workspaceId,
    plaintextHash,
    i64(profile.epoch),
  ]);
  const sealed = new Uint8Array(
    await crypto.subtle.encrypt(
      { name: 'AES-GCM', iv: nonce, additionalData: aad },
      key,
      plaintext,
    ),
  );
  const ciphertext = new Uint8Array([...nonce, ...sealed]);
  return {
    plaintextHash,
    plaintextSize: plaintext.length,
    ciphertext,
    serverHash: toHex(await sha256(ciphertext)),
    mimeType: blob.type || 'audio/webm',
  };
}

export async function uploadVoiceCapture(profile, recording) {
  if (!profile?.deviceId) throw new MobileSyncError('not_paired', '先加入同步再上传语音。');
  const next = await attachWorkspace(profile);
  const sealed = await encryptVoiceBlob(next, recording.blob);
  const chunkSize = 64 * 1024;
  const chunkCount = Math.ceil(sealed.ciphertext.length / chunkSize);
  const beginHash = await canonicalHash([
    'spanseek-sync-blob-begin-v1',
    sealed.serverHash,
    i64(sealed.ciphertext.length),
    i64(chunkSize),
  ]);
  await signedPost(next, '/v1/blobs/begin', 'write', beginHash, {
    blob_hash: sealed.serverHash,
    size: sealed.ciphertext.length,
    chunk_size: chunkSize,
  });
  for (let index = 0; index < chunkCount; index += 1) {
    const start = index * chunkSize;
    const chunk = sealed.ciphertext.subarray(
      start,
      Math.min(start + chunkSize, sealed.ciphertext.length),
    );
    const digest = toHex(await sha256(chunk));
    const chunkHash = await canonicalHash([
      'spanseek-sync-blob-chunk-v1',
      sealed.serverHash,
      u32(index),
      i64(chunk.length),
      digest,
    ]);
    await signedPost(next, '/v1/blobs/chunk', 'write', chunkHash, {
      blob_hash: sealed.serverHash,
      index,
      data: toBase64(chunk),
    });
  }
  const finalizeHash = await canonicalHash(['spanseek-sync-blob-finalize-v1', sealed.serverHash]);
  await signedPost(next, '/v1/blobs/finalize', 'write', finalizeHash, {
    blob_hash: sealed.serverHash,
  });
  const marker = `<!-- spanseek-voice:v1:${sealed.serverHash}:${sealed.plaintextHash}:${recording.durationMs}:${sealed.mimeType} -->`;
  const capture = {
    id: recording.id,
    objectId: recording.id,
    workspaceId: next.workspaceId,
    title: '语音捕捉',
    body: `${marker}\n（等待桌面转写）`,
    baseVersion: 0,
    updatedAt: new Date().toISOString(),
  };
  await pushCapture(next, capture);
  return { ...next, voiceHash: sealed.serverHash, plaintextHash: sealed.plaintextHash };
}

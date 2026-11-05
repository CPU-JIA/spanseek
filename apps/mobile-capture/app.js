import { MobileSyncError, pairDevice, syncMobile, uploadVoiceCapture } from './sync.js';

const DB_NAME = 'spanseek-mobile';
const DB_VERSION = 3;
const HANDOFF_FORMAT = 'spanseek-resume-handoff-v1';

const state = {
  workspaceId: '',
  captures: [],
  excerpts: [],
  activeId: null,
  shareUrl: '',
  scrollTimer: 0,
  restoring: false,
  syncProfile: null,
  syncBusy: true,
  syncConflicts: [],
  voiceRecordings: [],
  voiceRecorder: null,
  voiceChunks: [],
  voiceStartedAt: 0,
  voiceTimer: 0,
};

const dom = {
  captureForm: document.querySelector('#capture-form'),
  title: document.querySelector('#capture-title'),
  body: document.querySelector('#capture-body'),
  captureStatus: document.querySelector('#capture-status'),
  captureList: document.querySelector('#capture-list'),
  paste: document.querySelector('#paste-button'),
  export: document.querySelector('#export-button'),
  importInput: document.querySelector('#import-input'),
  readerEmpty: document.querySelector('#reader-empty'),
  reader: document.querySelector('#reader'),
  readerTitle: document.querySelector('#reader-title'),
  readerMeta: document.querySelector('#reader-meta'),
  readerBody: document.querySelector('#reader-body'),
  readerSource: document.querySelector('#reader-source'),
  excerptText: document.querySelector('#excerpt-text'),
  excerptStatus: document.querySelector('#excerpt-status'),
  excerptList: document.querySelector('#excerpt-list'),
  excerptCount: document.querySelector('#excerpt-count'),
  back: document.querySelector('#back-to-list'),
  saveExcerpt: document.querySelector('#save-excerpt'),
  connection: document.querySelector('#connection'),
  workspacePill: document.querySelector('#workspace-pill'),
  workspaceLabel: document.querySelector('#workspace-label'),
  workspaceInput: document.querySelector('#workspace-id'),
  bindWorkspace: document.querySelector('#bind-workspace'),
  workspaceStatus: document.querySelector('#workspace-status'),
  syncProfileLabel: document.querySelector('#sync-profile-label'),
  syncEndpoint: document.querySelector('#sync-endpoint'),
  syncDeviceName: document.querySelector('#sync-device-name'),
  syncInviteId: document.querySelector('#sync-invite-id'),
  syncCode: document.querySelector('#sync-code'),
  syncPair: document.querySelector('#sync-pair'),
  syncNow: document.querySelector('#sync-now'),
  syncStatus: document.querySelector('#sync-status'),
  syncConflicts: document.querySelector('#sync-conflicts'),
  voiceRecord: document.querySelector('#voice-record'),
  voiceTimer: document.querySelector('#voice-timer'),
  voiceStatus: document.querySelector('#voice-status'),
  voiceList: document.querySelector('#voice-list'),
};

function openDb() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains('captures')) {
        const store = db.createObjectStore('captures', { keyPath: 'id' });
        store.createIndex('createdAt', 'createdAt');
        store.createIndex('workspaceId', 'workspaceId');
      }
      if (!db.objectStoreNames.contains('excerpts')) {
        const store = db.createObjectStore('excerpts', { keyPath: 'id' });
        store.createIndex('createdAt', 'createdAt');
        store.createIndex('captureId', 'captureId');
        store.createIndex('workspaceId', 'workspaceId');
      }
      if (!db.objectStoreNames.contains('meta')) {
        db.createObjectStore('meta', { keyPath: 'key' });
      }
      if (!db.objectStoreNames.contains('voice')) {
        const store = db.createObjectStore('voice', { keyPath: 'id' });
        store.createIndex('createdAt', 'createdAt');
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function readAll(store) {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const request = db.transaction(store, 'readonly').objectStore(store).getAll();
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function write(store, value) {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction(store, 'readwrite');
    transaction.objectStore(store).put(value);
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error);
  });
}

async function setMeta(key, value) {
  await write('meta', { key, value });
}

async function getMeta(key) {
  const rows = await readAll('meta');
  return rows.find((row) => row.key === key)?.value;
}

function usableSyncProfile(value) {
  return value &&
    typeof value === 'object' &&
    typeof value.edPrivateJwk?.d === 'string' &&
    typeof value.xPrivateJwk?.d === 'string'
    ? value
    : null;
}

function sameSyncProfile(left, right) {
  return JSON.stringify(usableSyncProfile(left)) === JSON.stringify(usableSyncProfile(right));
}

function bindingChanged() {
  return new MobileSyncError(
    'sync_binding_changed',
    '同步设置或进度已有变化，未覆盖。请核对当前知识库后重试。',
  );
}

// The binding check and the write share a transaction, including writes from
// receive callbacks. Another PWA tab must not change their scope in between.
async function syncWrite(expected, workspaceId, stores, persist) {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction(['meta', ...stores], 'readwrite');
    const store = transaction.objectStore('meta');
    let failure = null;
    const abort = (error) => {
      failure = error;
      transaction.abort();
    };
    const profile = store.get('syncProfile');
    profile.onsuccess = () => {
      const active = store.get('activeWorkspaceId');
      active.onsuccess = () => {
        if (
          active.result?.value !== workspaceId ||
          !sameSyncProfile(profile.result?.value, expected)
        ) {
          abort(bindingChanged());
          return;
        }
        try {
          persist(transaction, abort);
        } catch (error) {
          abort(error);
        }
      };
    };
    transaction.oncomplete = () => {
      db.close();
      resolve();
    };
    transaction.onabort = () => {
      db.close();
      reject(failure || transaction.error || new Error('同步设置未能保存。'));
    };
  });
}

async function saveSyncProfile(expected, next, workspaceId) {
  await syncWrite(expected, workspaceId, [], (transaction) => {
    transaction.objectStore('meta').put({ key: 'syncProfile', value: next });
  });
}

async function saveSyncedCapture(profile, previous, changes) {
  let saved;
  const changed = () =>
    new MobileSyncError(
      'local_capture_changed',
      '这条本机内容已有变化，未覆盖。已重新读取，请再次同步并核对冲突。',
    );
  const baseline = (capture) =>
    capture &&
    JSON.stringify([
      capture.id,
      capture.workspaceId,
      capture.objectId || capture.id,
      capture.baseVersion || 0,
      capture.title,
      capture.body,
      Boolean(capture.trashed),
    ]);
  await syncWrite(profile, profile.workspaceId, ['captures'], (transaction, abort) => {
    const store = transaction.objectStore('captures');
    if (!previous) {
      saved = changes;
      if (changes.id === changes.objectId) {
        store.add(saved);
      } else {
        // The ordinary primary-key add is already unique. A collision-created
        // local ID also needs a logical-identity check against another tab.
        const request = store.index('workspaceId').openCursor(changes.workspaceId);
        request.onsuccess = () => {
          const cursor = request.result;
          if (!cursor) {
            store.add(saved);
          } else if ((cursor.value.objectId || cursor.value.id) === changes.objectId) {
            abort(changed());
          } else {
            cursor.continue();
          }
        };
      }
      return;
    }
    const request = store.get(previous.id);
    request.onsuccess = () => {
      if (baseline(request.result) !== baseline(previous)) {
        abort(changed());
        return;
      }
      // Reading position and other local fields may change independently.
      saved = { ...request.result, ...changes };
      store.put(saved);
    };
  });
  return saved;
}

function setStatus(node, message, error = false) {
  node.textContent = message;
  node.dataset.error = error ? 'true' : 'false';
}

function formatTime(value) {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? ''
    : new Intl.DateTimeFormat('zh-CN', {
        month: 'numeric',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      }).format(date);
}

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function validSource(value) {
  try {
    const url = new URL(value);
    return url.protocol === 'http:' || url.protocol === 'https:' ? url.toString() : '';
  } catch {
    return '';
  }
}

function validWorkspace(value) {
  return typeof value === 'string' && /^[A-Za-z0-9._:-]{1,200}$/.test(value.trim());
}

function shortWorkspace(value) {
  return value.length <= 14 ? value : `${value.slice(0, 6)}…${value.slice(-4)}`;
}

function activeCaptures() {
  return state.captures.filter(
    (capture) => capture.workspaceId === state.workspaceId && !capture.trashed,
  );
}

function activeExcerpts() {
  const ids = new Set(activeCaptures().map((capture) => capture.id));
  return state.excerpts.filter((excerpt) => ids.has(excerpt.captureId));
}

function renderWorkspace() {
  dom.workspacePill.textContent = shortWorkspace(state.workspaceId);
  dom.workspacePill.title = `当前知识库：${state.workspaceId}`;
  dom.workspaceLabel.textContent = state.workspaceId;
  if (document.activeElement !== dom.workspaceInput) dom.workspaceInput.value = state.workspaceId;
}

function renderCaptures() {
  dom.captureList.replaceChildren();
  const captures = activeCaptures();
  if (captures.length === 0) {
    dom.captureList.append(element('li', 'empty', '当前知识库还没有捕捉。先记下第一条。'));
    return;
  }
  for (const capture of captures) {
    const item = element('li');
    const button = element('button', 'card');
    button.type = 'button';
    button.dataset.captureId = capture.id;
    button.append(
      element('h3', '', capture.title || '未命名捕捉'),
      element('p', '', capture.body),
      element(
        'p',
        'meta',
        `${formatTime(capture.createdAt)}${capture.sourceUrl ? ' · 来自分享' : ''} · v${capture.baseVersion || 0}`,
      ),
    );
    button.addEventListener('click', () => openReader(capture.id));
    item.append(button);
    dom.captureList.append(item);
  }
}

function renderExcerpts() {
  dom.excerptList.replaceChildren();
  const excerpts = activeExcerpts();
  dom.excerptCount.textContent = String(excerpts.length);
  if (excerpts.length === 0) {
    dom.excerptList.append(
      element('li', 'empty', '当前知识库还没有摘录。阅读时选中文字即可保存。'),
    );
    return;
  }
  for (const excerpt of excerpts) {
    const item = element('li');
    const card = element('div', 'card');
    card.append(
      element('p', '', excerpt.text),
      element(
        'p',
        'meta',
        `${excerpt.captureTitle || '未命名'} · ${formatTime(excerpt.createdAt)}`,
      ),
    );
    item.append(card);
    dom.excerptList.append(item);
  }
}

function renderReader() {
  const capture = state.captures.find(
    (entry) => entry.id === state.activeId && entry.workspaceId === state.workspaceId,
  );
  dom.readerEmpty.hidden = Boolean(capture);
  dom.reader.hidden = !capture;
  if (!capture) return;
  dom.readerTitle.textContent = capture.title || '未命名捕捉';
  dom.readerMeta.textContent = `${formatTime(capture.createdAt)} · 桌面版本 v${capture.baseVersion || 0}`;
  dom.readerBody.textContent = capture.body;
  const source = validSource(capture.sourceUrl);
  dom.readerSource.hidden = !source;
  if (source) {
    dom.readerSource.href = source;
    dom.readerSource.textContent = source;
  }
  dom.excerptText.value = '';
  setStatus(dom.excerptStatus, '');
  state.restoring = true;
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      window.scrollTo({ top: capture.readingPosition || 0, behavior: 'auto' });
      requestAnimationFrame(() => {
        state.restoring = false;
      });
    });
  });
}

function switchView(name) {
  for (const view of document.querySelectorAll('.view')) {
    view.hidden = view.dataset.view !== name;
  }
  for (const tab of document.querySelectorAll('.tab')) {
    tab.classList.toggle('active', tab.dataset.target === name);
  }
  if (name === 'read') {
    renderReader();
  } else {
    window.scrollTo({ top: 0, behavior: 'instant' });
  }
}

async function openReader(id) {
  state.activeId = id;
  switchView('read');
}

async function saveCapture(event) {
  event?.preventDefault();
  const body = dom.body.value.trim();
  if (!body) {
    setStatus(dom.captureStatus, '内容不能为空。', true);
    dom.body.focus();
    return;
  }
  const now = new Date().toISOString();
  const objectId = crypto.randomUUID();
  const capture = {
    id: objectId,
    objectId,
    workspaceId: state.workspaceId,
    baseVersion: 0,
    title: dom.title.value.trim() || body.split('\n')[0].slice(0, 60),
    body,
    sourceUrl: validSource(state.shareUrl),
    createdAt: now,
    updatedAt: now,
    readingPosition: 0,
  };
  await write('captures', capture);
  state.captures.unshift(capture);
  renderCaptures();
  dom.captureForm.reset();
  state.shareUrl = '';
  setStatus(dom.captureStatus, '已保存到当前知识库，离线可读。');
  if (state.syncProfile?.deviceId && navigator.onLine) void syncNow(true);
}

function selectedReaderText() {
  const selection = window.getSelection();
  if (!selection || selection.rangeCount === 0) return '';
  const range = selection.getRangeAt(0);
  if (!dom.readerBody.contains(range.commonAncestorContainer)) return '';
  return selection.toString().trim();
}

async function saveExcerpt() {
  const capture = state.captures.find(
    (entry) => entry.id === state.activeId && entry.workspaceId === state.workspaceId,
  );
  if (!capture) return;
  const text = selectedReaderText() || dom.excerptText.value.trim();
  if (!text) {
    setStatus(dom.excerptStatus, '先选中正文或输入摘录。', true);
    return;
  }
  const excerpt = {
    id: crypto.randomUUID(),
    workspaceId: state.workspaceId,
    captureId: capture.id,
    captureTitle: capture.title,
    text,
    createdAt: new Date().toISOString(),
  };
  await write('excerpts', excerpt);
  state.excerpts.unshift(excerpt);
  renderExcerpts();
  dom.excerptText.value = '';
  setStatus(dom.excerptStatus, '摘录已保存。');
}

async function persistReadingPosition() {
  const capture = state.captures.find(
    (entry) => entry.id === state.activeId && entry.workspaceId === state.workspaceId,
  );
  if (!capture) return;
  capture.readingPosition = window.scrollY;
  capture.updatedAt = new Date().toISOString();
  await write('captures', capture);
}

function onScroll() {
  if (state.restoring || !state.activeId || document.querySelector('#view-read').hidden) return;
  clearTimeout(state.scrollTimer);
  state.scrollTimer = window.setTimeout(() => {
    persistReadingPosition().catch(() => {});
  }, 260);
}

function exportData() {
  const captures = activeCaptures();
  const activeIds = new Set(captures.map((capture) => capture.id));
  const payload = {
    format: HANDOFF_FORMAT,
    workspace_id: state.workspaceId,
    exported_at: new Date().toISOString(),
    objects: captures.map((capture) => ({
      object_id: capture.objectId || capture.id,
      title: capture.title,
      body: capture.body,
      source_url: capture.sourceUrl || '',
      base_version: capture.baseVersion || 0,
      updated_at: capture.updatedAt || capture.createdAt,
      reading_position: { kind: 'scroll', position: capture.readingPosition || 0 },
    })),
    excerpts: state.excerpts.filter((excerpt) => activeIds.has(excerpt.captureId)),
  };
  const blob = new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = `spanseek-resume-${state.workspaceId.slice(0, 12)}-${new Date().toISOString().slice(0, 10)}.json`;
  link.click();
  URL.revokeObjectURL(url);
  setStatus(dom.captureStatus, '续读包已导出；包含当前知识库的正文、阅读位置与摘录。');
}

function captureFromObject(object, workspaceId) {
  if (!object || typeof object !== 'object') return null;
  const objectId = object.object_id || object.id;
  const body = object.body;
  if (typeof objectId !== 'string' || typeof body !== 'string') return null;
  const now = new Date().toISOString();
  const locator = object.reading_position;
  const readingPosition =
    locator && locator.kind === 'scroll' && Number.isFinite(locator.position)
      ? Math.max(0, Math.trunc(locator.position))
      : 0;
  return {
    id: objectId,
    objectId,
    workspaceId,
    baseVersion: Number.isInteger(object.base_version) ? object.base_version : 0,
    title:
      typeof object.title === 'string' && object.title.trim()
        ? object.title
        : body.split('\n')[0].slice(0, 60),
    body,
    sourceUrl: validSource(object.source_url || ''),
    createdAt: object.updated_at || now,
    updatedAt: object.updated_at || now,
    readingPosition,
  };
}

function importBundle(payload) {
  if (!payload || typeof payload !== 'object') throw new Error('format');
  if (payload.format === HANDOFF_FORMAT) {
    if (!validWorkspace(payload.workspace_id) || !Array.isArray(payload.objects))
      throw new Error('format');
    return {
      workspaceId: payload.workspace_id.trim(),
      captures: payload.objects
        .map((object) => captureFromObject(object, payload.workspace_id.trim()))
        .filter(Boolean),
      excerpts: Array.isArray(payload.excerpts) ? payload.excerpts : [],
    };
  }
  if (
    payload.format === 'spanseek-mobile-export-v1' ||
    payload.format === 'spanseek-mobile-export-v2'
  ) {
    const workspaceId = validWorkspace(payload.workspace_id)
      ? payload.workspace_id.trim()
      : state.workspaceId;
    if (!Array.isArray(payload.captures) || !Array.isArray(payload.excerpts))
      throw new Error('format');
    return {
      workspaceId,
      captures: payload.captures
        .filter((capture) => typeof capture.id === 'string' && typeof capture.body === 'string')
        .map((capture) => {
          const now = new Date().toISOString();
          return {
            ...capture,
            workspaceId,
            objectId: capture.objectId || capture.id,
            baseVersion: Number.isInteger(capture.baseVersion) ? capture.baseVersion : 0,
            title:
              typeof capture.title === 'string'
                ? capture.title
                : capture.body.split('\n')[0].slice(0, 60),
            sourceUrl: validSource(capture.sourceUrl || ''),
            createdAt: typeof capture.createdAt === 'string' ? capture.createdAt : now,
            updatedAt: typeof capture.updatedAt === 'string' ? capture.updatedAt : now,
            readingPosition: Number.isFinite(capture.readingPosition)
              ? Math.max(0, Math.trunc(capture.readingPosition))
              : 0,
          };
        }),
      excerpts: payload.excerpts,
    };
  }
  throw new Error('format');
}

async function importData(event) {
  const file = event.target.files?.[0];
  if (!file) return;
  if (state.syncBusy) {
    setStatus(dom.captureStatus, '同步操作进行中，请完成后再导入。', true);
    event.target.value = '';
    return;
  }
  state.syncBusy = true;
  renderSync();
  try {
    const bundle = importBundle(JSON.parse(await file.text()));
    const hasCurrent = activeCaptures().length > 0;
    if (bundle.workspaceId !== state.workspaceId && hasCurrent) {
      const approved = window.confirm(
        `导入包属于另一个知识库（${shortWorkspace(bundle.workspaceId)}）。当前库会原样保留，但界面将切换到导入库。继续？`,
      );
      if (!approved) {
        setStatus(dom.captureStatus, '已取消导入；当前知识库未改变。', true);
        return;
      }
    }
    state.workspaceId = bundle.workspaceId;
    await setMeta('activeWorkspaceId', state.workspaceId);
    for (const capture of bundle.captures) {
      capture.workspaceId = bundle.workspaceId;
      await write('captures', capture);
    }
    for (const excerpt of bundle.excerpts) {
      if (typeof excerpt.id !== 'string' || typeof excerpt.text !== 'string') continue;
      excerpt.workspaceId = bundle.workspaceId;
      await write('excerpts', excerpt);
    }
    await load();
    setStatus(dom.captureStatus, `已导入 ${bundle.captures.length} 条续读内容。`);
  } catch {
    setStatus(dom.captureStatus, '导入文件格式不正确。', true);
  } finally {
    event.target.value = '';
    state.syncBusy = false;
    renderSync();
  }
  if (state.syncProfile?.deviceId && navigator.onLine) void syncNow(true);
}

function renderSync() {
  const profile = state.syncProfile;
  dom.syncProfileLabel.textContent = !profile
    ? '未加入同步'
    : profile.deviceId
      ? `已配对 ${profile.deviceId.slice(0, 8)} · 库 ${shortWorkspace(profile.workspaceId || '')} · epoch ${profile.epoch || 0}`
      : '设备身份已生成，尚未配对';
  if (profile?.endpoint && document.activeElement !== dom.syncEndpoint) {
    dom.syncEndpoint.value = profile.endpoint;
  }
  if (profile?.displayName && document.activeElement !== dom.syncDeviceName) {
    dom.syncDeviceName.value = profile.displayName;
  }
  dom.syncConflicts.replaceChildren();
  for (const conflict of state.syncConflicts) {
    const item = element('li');
    const card = element('div', 'card');
    card.append(
      element('p', '', `同步冲突：${conflict.objectId.slice(0, 8)}`),
      element('p', 'meta', conflict.message || '需要人工比较这条内容。'),
    );
    item.append(card);
    dom.syncConflicts.append(item);
  }
  dom.syncNow.disabled = state.syncBusy || !profile?.deviceId;
  dom.syncNow.textContent = profile?.hasMore ? '继续同步' : '同步现在';
  dom.syncPair.disabled = state.syncBusy;
  dom.bindWorkspace.disabled = state.syncBusy;
  dom.importInput.disabled = state.syncBusy;
}

async function pairSync() {
  if (state.syncBusy) return;
  const endpoint = dom.syncEndpoint.value.trim();
  const displayName = dom.syncDeviceName.value.trim() || '手机';
  const inviteId = dom.syncInviteId.value.trim();
  const code = dom.syncCode.value.trim();
  if (!validWorkspace(state.workspaceId)) {
    setStatus(dom.syncStatus, '先绑定桌面知识库 ID。', true);
    return;
  }
  if (!endpoint || !inviteId || !code) {
    setStatus(dom.syncStatus, '服务器地址、配对 ID 和配对码都不能为空。', true);
    return;
  }
  state.syncBusy = true;
  renderSync();
  const expected = structuredClone(state.syncProfile);
  const workspaceId = state.workspaceId;
  try {
    let profile = structuredClone(expected);
    if (!profile?.deviceId) {
      profile = await pairDevice(profile, { endpoint, displayName, inviteId, code });
    }
    if (profile.workspaceId && profile.workspaceId !== workspaceId) {
      profile.keys = {};
      profile.cursor = 0;
      profile.epoch = 0;
      profile.hasMore = false;
    }
    profile.endpoint = endpoint;
    profile.displayName = displayName;
    profile.workspaceId = workspaceId;
    await saveSyncProfile(expected, profile, workspaceId);
    state.syncProfile = profile;
    dom.syncInviteId.value = '';
    dom.syncCode.value = '';
    setStatus(dom.syncStatus, '已配对。请在已有设备上授权密钥，然后点“同步现在”。');
    renderSync();
  } catch (error) {
    if (error?.code === 'sync_binding_changed') await load();
    setStatus(dom.syncStatus, error.message || '配对失败。', true);
  } finally {
    state.syncBusy = false;
    renderSync();
  }
}

async function syncNow(silent = false) {
  if (state.syncBusy) return;
  const workspaceId = state.workspaceId;
  const profile = structuredClone(state.syncProfile);
  function showStatus(message, { error = false, attention = error } = {}) {
    if (state.workspaceId !== workspaceId) return;
    setStatus(dom.syncStatus, message, error);
    if (
      !silent &&
      attention &&
      !dom.syncStatus.closest('details:not([open]), [hidden]') &&
      dom.syncStatus.getClientRects().length
    ) {
      dom.syncStatus.scrollIntoView({ block: 'center', behavior: 'instant' });
    }
  }
  function checkBinding() {
    if (state.workspaceId !== workspaceId || !sameSyncProfile(state.syncProfile, profile)) {
      throw bindingChanged();
    }
  }
  if (!profile?.deviceId) {
    if (!silent) showStatus('先加入同步。', { error: true });
    return;
  }
  if (profile.workspaceId !== state.workspaceId) {
    if (!silent) showStatus('当前知识库与同步绑定不一致；切回绑定的库后再同步。', { error: true });
    return;
  }
  if (!navigator.onLine) {
    if (!silent) showStatus('当前离线；本地内容保留，联网后再同步。', { error: true });
    return;
  }
  state.syncBusy = true;
  renderSync();
  const conflicts = [];
  try {
    const result = await syncMobile(profile, state.captures, {
      onRemote: async (note) => {
        checkBinding();
        let capture = state.captures.find(
          (entry) =>
            entry.workspaceId === profile.workspaceId &&
            (entry.objectId || entry.id) === note.objectId,
        );
        if (
          !capture &&
          state.captures.some((entry) => (entry.objectId || entry.id) === note.objectId)
        ) {
          conflicts.push({
            objectId: note.objectId,
            message: '同一对象 ID 属于另一个本机知识库，未覆盖。',
          });
          return false;
        }
        if (!capture) {
          capture = {
            id: state.captures.some((entry) => entry.id === note.objectId)
              ? crypto.randomUUID()
              : note.objectId,
            objectId: note.objectId,
            workspaceId: profile.workspaceId,
            baseVersion: note.version,
            title: note.title,
            body: note.body,
            sourceUrl: '',
            createdAt: note.updatedAt,
            updatedAt: note.updatedAt,
            readingPosition: 0,
          };
          await saveSyncedCapture(profile, null, capture);
          state.captures.push(capture);
          return true;
        }
        if (
          (capture.baseVersion || 0) === 0 &&
          (capture.title !== note.title || capture.body !== note.body)
        ) {
          conflicts.push({
            objectId: note.objectId,
            message: '这条内容在手机和桌面同时改动，先保留手机版本。',
          });
          return false;
        }
        if ((capture.baseVersion || 0) >= note.version) return false;
        const updated = await saveSyncedCapture(profile, structuredClone(capture), {
          title: note.title,
          body: note.body,
          updatedAt: note.updatedAt,
          baseVersion: note.version,
          trashed: false,
        });
        Object.assign(capture, updated);
        return true;
      },
      onDelete: async (deleted) => {
        checkBinding();
        const capture = state.captures.find(
          (entry) =>
            entry.workspaceId === profile.workspaceId &&
            (entry.objectId || entry.id) === deleted.objectId,
        );
        if (!capture) return;
        if ((capture.baseVersion || 0) >= deleted.version) return;
        const updated = await saveSyncedCapture(profile, structuredClone(capture), {
          trashed: true,
          baseVersion: deleted.version,
        });
        Object.assign(capture, updated);
      },
      onVersion: async (objectId, version) => {
        checkBinding();
        const capture = state.captures.find(
          (entry) =>
            entry.workspaceId === profile.workspaceId && (entry.objectId || entry.id) === objectId,
        );
        if (!capture) return;
        const updated = await saveSyncedCapture(profile, structuredClone(capture), {
          baseVersion: version,
        });
        Object.assign(capture, updated);
      },
      onConflict: (conflict) => {
        conflicts.push(conflict);
      },
    });
    checkBinding();
    const nextProfile = { ...result.profile, hasMore: result.hasMore };
    await saveSyncProfile(profile, nextProfile, workspaceId);
    state.syncProfile = nextProfile;
    state.syncConflicts = conflicts;
    if (conflicts.some((conflict) => conflict.code === 'local_capture_changed')) {
      state.captures = await readAll('captures');
    }
    state.captures.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    renderCaptures();
    renderReader();
    renderSync();
    showStatus(
      result.receiveBlocked
        ? `同步未完成：已接收 ${result.remoteChanges.length} 条，已保留未处理位置。请处理提示后重试。`
        : result.hasMore
          ? `本轮已接收 ${result.remoteChanges.length} 条，仍有内容待接收。点击“继续同步”接收剩余内容。`
          : conflicts.length
            ? `同步完成，${conflicts.length} 条冲突需要人工处理。`
            : `同步完成：接收 ${result.remoteChanges.length} 条，发送后已更新本机版本。`,
      {
        error: result.receiveBlocked || conflicts.length > 0,
        attention: result.receiveBlocked || result.hasMore || conflicts.length > 0,
      },
    );
  } catch (error) {
    const waiting =
      error?.code === 'key_not_granted' ||
      error?.code === 'key_not_found' ||
      error?.code === 'missing_workspace_key';
    if (error?.code === 'sync_binding_changed' || error?.code === 'local_capture_changed') {
      await load();
    }
    showStatus(
      waiting
        ? '等待已有设备授权同步密钥，然后再点“同步现在”。'
        : error.message || '同步失败；本地内容未丢失。',
      { error: true },
    );
  } finally {
    state.syncBusy = false;
    renderSync();
  }
}

async function bindWorkspace() {
  if (state.syncBusy) return;
  const workspaceId = dom.workspaceInput.value.trim();
  if (!validWorkspace(workspaceId)) {
    setStatus(
      dom.workspaceStatus,
      '知识库 ID 只能包含字母、数字、点、下划线、冒号或连字符。',
      true,
    );
    return;
  }
  if (workspaceId === state.workspaceId) {
    setStatus(dom.workspaceStatus, '已经是当前知识库。');
    return;
  }
  if (activeCaptures().length > 0) {
    const approved = window.confirm('切换后旧知识库数据会保留但不再显示。确认切换？');
    if (!approved) {
      setStatus(dom.workspaceStatus, '已取消切换。');
      return;
    }
  }
  state.syncBusy = true;
  renderSync();
  try {
    await setMeta('activeWorkspaceId', workspaceId);
    state.workspaceId = workspaceId;
    state.activeId = null;
    await load();
    setStatus(dom.workspaceStatus, '已切换；不同知识库不会混合显示。');
  } finally {
    state.syncBusy = false;
    renderSync();
  }
  if (state.syncProfile?.deviceId && navigator.onLine) void syncNow(true);
}

function applyShareTarget() {
  const params = new URLSearchParams(window.location.search);
  const text = params.get('text') || '';
  const url = params.get('url') || '';
  const title = params.get('title') || '';
  if (!text && !url && !title) return;
  state.shareUrl = url;
  dom.title.value = title;
  dom.body.value = text || url;
  setStatus(dom.captureStatus, '来自分享。确认内容后点“保存捕捉”。');
  window.history.replaceState({}, '', window.location.pathname);
}

function updateConnection() {
  const online = navigator.onLine;
  dom.connection.textContent = online ? '在线' : '离线';
  dom.connection.dataset.online = online ? 'true' : 'false';
}

async function load() {
  const storedWorkspace = await getMeta('activeWorkspaceId');
  const storedProfile = await getMeta('syncProfile');
  state.syncProfile = usableSyncProfile(storedProfile);
  if (validWorkspace(storedWorkspace)) state.workspaceId = storedWorkspace.trim();
  if (!state.workspaceId) {
    state.workspaceId = `local-${crypto.randomUUID()}`;
    await setMeta('activeWorkspaceId', state.workspaceId);
  }
  const [captures, excerpts] = await Promise.all([readAll('captures'), readAll('excerpts')]);
  for (const capture of captures) {
    if (!validWorkspace(capture.workspaceId)) {
      capture.workspaceId = state.workspaceId;
      capture.objectId = capture.objectId || capture.id;
      capture.baseVersion = capture.baseVersion || 0;
      await write('captures', capture);
    }
  }
  for (const excerpt of excerpts) {
    if (!validWorkspace(excerpt.workspaceId)) {
      const capture = captures.find((entry) => entry.id === excerpt.captureId);
      excerpt.workspaceId = capture?.workspaceId || state.workspaceId;
      await write('excerpts', excerpt);
    }
  }
  state.captures = captures.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  state.excerpts = excerpts.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  if (state.activeId && !activeCaptures().some((entry) => entry.id === state.activeId)) {
    state.activeId = null;
  }
  renderWorkspace();
  renderCaptures();
  renderExcerpts();
  renderReader();
  renderSync();
}

function voiceClock(ms) {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
}

function renderVoiceRecordings() {
  dom.voiceList.textContent = '';
  for (const recording of state.voiceRecordings) {
    const item = document.createElement('li');
    item.className = 'card voice-item';
    const copy = document.createElement('div');
    const title = document.createElement('strong');
    title.textContent = new Date(recording.createdAt).toLocaleString();
    const meta = document.createElement('p');
    meta.className = 'meta';
    meta.textContent = `${voiceClock(recording.durationMs)} · ${Math.round(recording.blob.size / 1024)} KB · ${
      recording.uploadedAt ? '已上传' : '待上传'
    }`;
    copy.append(title, meta);
    const download = document.createElement('button');
    download.type = 'button';
    download.className = 'ghost';
    download.textContent = '导出音频';
    download.addEventListener('click', () => {
      const url = URL.createObjectURL(recording.blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = `spanseek-voice-${recording.createdAt.replace(/[:.]/g, '-')}.webm`;
      anchor.click();
      setTimeout(() => URL.revokeObjectURL(url), 2000);
    });
    const upload = document.createElement('button');
    upload.type = 'button';
    upload.className = 'ghost';
    upload.textContent = '上传到桌面';
    upload.addEventListener('click', () => {
      if (!state.syncProfile?.deviceId) {
        setStatus(dom.voiceStatus, '先在“知识库绑定与续读”里加入同步。', true);
        return;
      }
      upload.disabled = true;
      setStatus(dom.voiceStatus, '正在加密上传…');
      uploadVoiceCapture(state.syncProfile, recording)
        .then(async (next) => {
          state.syncProfile = next;
          await setMeta('syncProfile', next);
          recording.uploadedAt = new Date().toISOString();
          await write('voice', recording);
          renderVoiceRecordings();
          setStatus(dom.voiceStatus, '音频已上传；桌面同步后生成语音条目。');
        })
        .catch((error) => setStatus(dom.voiceStatus, `上传失败：${error.message}`, true))
        .finally(() => {
          upload.disabled = false;
        });
    });
    const actions = document.createElement('div');
    actions.className = 'row';
    actions.append(download, upload);
    item.append(copy, actions);
    dom.voiceList.append(item);
  }
}

async function loadVoiceRecordings() {
  state.voiceRecordings = (await readAll('voice')).sort((a, b) =>
    b.createdAt.localeCompare(a.createdAt),
  );
  renderVoiceRecordings();
}

async function startVoiceRecording() {
  if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === 'undefined') {
    setStatus(dom.voiceStatus, '当前浏览器不支持录音。', true);
    return;
  }
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  const recorder = new MediaRecorder(stream, {
    mimeType: MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
      ? 'audio/webm;codecs=opus'
      : 'audio/webm',
  });
  state.voiceChunks = [];
  state.voiceRecorder = recorder;
  state.voiceStartedAt = performance.now();
  recorder.addEventListener('dataavailable', (event) => {
    if (event.data.size > 0) state.voiceChunks.push(event.data);
  });
  recorder.addEventListener('stop', () => {
    stream.getTracks().forEach((track) => track.stop());
  });
  recorder.start(1000);
  dom.voiceRecord.textContent = '停止录音';
  dom.voiceRecord.classList.add('recording');
  setStatus(dom.voiceStatus, '正在录音…');
  state.voiceTimer = window.setInterval(() => {
    dom.voiceTimer.textContent = voiceClock(performance.now() - state.voiceStartedAt);
  }, 200);
}

async function stopVoiceRecording() {
  const recorder = state.voiceRecorder;
  if (!recorder) return;
  const durationMs = Math.round(performance.now() - state.voiceStartedAt);
  const stopped = new Promise((resolve) =>
    recorder.addEventListener('stop', resolve, { once: true }),
  );
  recorder.stop();
  await stopped;
  window.clearInterval(state.voiceTimer);
  state.voiceRecorder = null;
  dom.voiceRecord.textContent = '开始录音';
  dom.voiceRecord.classList.remove('recording');
  dom.voiceTimer.textContent = '0:00';
  const blob = new Blob(state.voiceChunks, { type: recorder.mimeType || 'audio/webm' });
  state.voiceChunks = [];
  if (blob.size === 0) {
    setStatus(dom.voiceStatus, '没有录到音频。', true);
    return;
  }
  const recording = {
    id: crypto.randomUUID(),
    createdAt: new Date().toISOString(),
    durationMs,
    mimeType: blob.type,
    blob,
  };
  await write('voice', recording);
  state.voiceRecordings.unshift(recording);
  renderVoiceRecordings();
  setStatus(dom.voiceStatus, '录音已保存在本机。');
}

dom.captureForm.addEventListener('submit', (event) => {
  saveCapture(event).catch(() =>
    setStatus(dom.captureStatus, '保存失败：浏览器存储不可用。', true),
  );
});
dom.voiceRecord.addEventListener('click', () => {
  const action = state.voiceRecorder ? stopVoiceRecording() : startVoiceRecording();
  action.catch((error) =>
    setStatus(
      dom.voiceStatus,
      error?.name === 'NotAllowedError' ? '麦克风权限被拒绝。' : '录音失败。',
      true,
    ),
  );
});
dom.paste.addEventListener('click', async () => {
  try {
    const text = await navigator.clipboard.readText();
    if (text) {
      dom.body.value = text;
      dom.body.focus();
    }
  } catch {
    setStatus(dom.captureStatus, '无法读取剪贴板，请手动粘贴。', true);
  }
});
dom.saveExcerpt.addEventListener('click', () => {
  saveExcerpt().catch(() => setStatus(dom.excerptStatus, '摘录保存失败。', true));
});
dom.back.addEventListener('click', () => switchView('capture'));
dom.export.addEventListener('click', exportData);
dom.importInput.addEventListener('change', (event) => {
  importData(event).catch(() => setStatus(dom.captureStatus, '导入失败。', true));
});
dom.bindWorkspace.addEventListener('click', () => {
  bindWorkspace().catch(() => setStatus(dom.workspaceStatus, '切换失败：浏览器存储不可用。', true));
});
dom.syncPair.addEventListener('click', () => {
  pairSync().catch(() => setStatus(dom.syncStatus, '配对失败。', true));
});
dom.syncNow.addEventListener('click', () => {
  syncNow().catch(() => setStatus(dom.syncStatus, '同步失败。', true));
});
dom.workspacePill.addEventListener('click', () => {
  document.querySelector('.bind-box').open = true;
  dom.workspaceInput.focus();
});
for (const tab of document.querySelectorAll('.tab')) {
  tab.addEventListener('click', () => switchView(tab.dataset.target));
}
window.addEventListener('scroll', onScroll, { passive: true });
window.addEventListener('online', () => {
  updateConnection();
  if (state.syncProfile?.deviceId) void syncNow(true);
});
window.addEventListener('offline', updateConnection);

updateConnection();
applyShareTarget();
renderSync();
load()
  .catch(() => setStatus(dom.captureStatus, '本地存储不可用。', true))
  .finally(() => {
    state.syncBusy = false;
    renderSync();
    if (state.syncProfile?.deviceId && navigator.onLine) void syncNow(true);
  });
loadVoiceRecordings().catch(() => setStatus(dom.voiceStatus, '录音列表读取失败。', true));

if ('serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    navigator.serviceWorker.register('./sw.js').catch(() => {});
  });
}

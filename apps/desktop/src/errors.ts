import type { ApiErrorCode, ApiErrorDetails, OperationKind } from '@pkms/contracts';
import { activeLanguage, type Language } from './language.ts';

/**
 * Interface text for a kernel error code.
 *
 * The kernel answers with a code plus a Chinese sentence, and it has no idea
 * which language the interface is showing. This table is the interface's own
 * single source: every code the kernel can send has one sentence per language,
 * so an English session never falls back to Chinese text.
 *
 * The Chinese strings mirror `ApiError::new` in `crates/pkms-app/src/ipc.rs`.
 * If that table changes, this one changes with it.
 */
const API_MESSAGES: Record<ApiErrorCode, [zh: string, en: string]> = {
  backend_starting: ['正在准备本地存储，请稍候。', 'Preparing local storage. Please wait.'],
  query_expired: [
    '列表已更新，请刷新后继续浏览。',
    'The list has changed. Refresh to continue browsing.',
  ],
  invalid_input: [
    '输入内容不符合要求，请检查后重试。',
    'The input is not acceptable. Check it and try again.',
  ],
  invalid_document: [
    '正文包含不支持的结构或格式。草稿仍需保留，请检查内容。',
    'The document contains an unsupported structure or format. Keep the draft and check the content.',
  ],
  unsupported_api: [
    '应用界面与服务版本不兼容，请重新启动应用。',
    'The interface and the local service are not compatible. Restart the application.',
  ],
  instance_mismatch: [
    '当前工作区已改变，请回到对应工作区后重试。',
    'The active workspace changed. Return to that workspace and try again.',
  ],
  app_state_conflict: [
    '界面设置已更新，请重新读取后再修改。',
    'Interface settings were updated. Read them again before changing them.',
  ],
  revision_conflict: [
    '这篇笔记已有更新。请比较版本后决定如何保存。',
    'This note has newer changes. Compare the versions before saving.',
  ],
  metadata_conflict: [
    '对象信息已有更新。请刷新后确认本次整理操作。',
    'The object was updated elsewhere. Refresh before repeating this change.',
  ],
  idempotency_mismatch: [
    '本次操作的重试内容不一致，已停止提交。',
    'This retry does not match the original request, so it was not submitted.',
  ],
  not_found: ['未找到所需内容或授权已失效。', 'The item was not found, or the permission expired.'],
  object_trashed: [
    '对象已在回收站，请先恢复再修改。',
    'The object is in the trash. Restore it before editing.',
  ],
  storage_busy: [
    '工作区正被占用，请关闭其他使用此工作区的程序后重试。',
    'The workspace is in use. Close the other program using it and try again.',
  ],
  storage_full: [
    '磁盘空间不足。请保留草稿，释放空间后重试。',
    'The disk is full. Keep the draft, free up space, then try again.',
  ],
  io_failed: [
    '无法读取或写入文件，请检查文件位置和访问权限。',
    'The file could not be read or written. Check its location and permissions.',
  ],
  integrity_failed: [
    '内容完整性检查未通过，请保留原文件并检查备份。',
    'The integrity check failed. Keep the original file and check your backup.',
  ],
  unsupported_database: [
    '这个知识库由更新版本的 spanseek 创建，原数据未被替换，当前版本不会写入或改动它。请更新 spanseek 后重试。',
    'This library was created by a newer spanseek. Your data was left unchanged and this build will not write to it; update spanseek and try again.',
  ],
  migration_failed: [
    '工作区升级检查未通过，请保留原库及升级前快照。',
    'The workspace upgrade check failed. Keep the library and its pre-upgrade snapshot.',
  ],
  input_unavailable: [
    '所选文件或位置已改变，或授权已过期，请重新选择。',
    'The chosen file or location changed, or the permission expired. Choose it again.',
  ],
  output_exists: [
    '目标文件已存在。请换一个名称或保存位置。',
    'The destination file already exists. Choose another name or location.',
  ],
  internal: [
    '操作未完成，请保留当前内容并查看诊断信息。',
    'The action did not finish. Keep your content and check diagnostics.',
  ],
};

const CITATION_FAILURE_MESSAGES: Record<string, [zh: string, en: string]> = {
  csl_required: ['请选择 .csl 样式文件。', 'Choose a .csl style file.'],
  csl_unreadable: [
    '无法读取 CSL 样式文件，请检查文件位置和权限。',
    'The CSL style could not be read. Check its location and permissions.',
  ],
  csl_too_large: ['CSL 样式文件过大，无法导入。', 'The CSL style file is too large to import.'],
  csl_not_utf8: ['CSL 样式必须使用 UTF-8 编码。', 'The CSL style must use UTF-8 encoding.'],
  csl_invalid_style: [
    'CSL 样式格式无效，请检查 XML 和 CSL 版本。',
    'The CSL style is invalid. Check its XML and CSL version.',
  ],
  csl_invalid_identity: [
    'CSL 样式缺少有效 id，无法保存。',
    'The CSL style has no valid id, so it cannot be saved.',
  ],
  csl_invalid_parent: [
    '依赖样式的 independent-parent 链接无效。',
    'The dependent style has an invalid independent-parent link.',
  ],
  csl_builtin_id_reserved: [
    '内置样式的 id 为保留值，请为自定义样式使用其他 id。',
    'That id is reserved by a built-in style. Use another id for the custom style.',
  ],
  csl_parent_missing: [
    '找不到依赖样式的父样式，请先导入独立父样式。',
    'The dependent style’s parent is missing. Import the independent parent first.',
  ],
  csl_parent_cycle: [
    'CSL 依赖样式形成循环，无法渲染。',
    'The dependent CSL styles form a cycle and cannot be rendered.',
  ],
  csl_parent_limit: [
    'CSL 依赖层级过深，无法渲染。',
    'The CSL dependency chain is too deep to render.',
  ],
  csl_builtin_readonly: ['内置 CSL 样式不能删除。', 'Built-in CSL styles cannot be deleted.'],
  csl_style_in_use: [
    '该样式仍被依赖样式使用，请先删除依赖样式。',
    'A dependent style still uses this parent. Delete the dependent style first.',
  ],
};

const SYNC_FAILURE_MESSAGES: Record<string, [zh: string, en: string]> = {
  sync_config_local_changed: [
    '本机配置在核对后又发生了变化，本次未覆盖。请重新读取并核对冲突后再选择。',
    'The local configuration changed after your review and was not overwritten. Refresh the comparison before choosing again.',
  ],
  sync_config_conflict_changed: [
    '远端状态在核对后发生了变化，本次未覆盖。请重新拉取并核对最新冲突。',
    'The remote state changed after your review and was not overwritten. Pull again and review the latest conflict.',
  ],
  insufficient_role: [
    '当前成员是只读者，只能读取和拉取；请让所有者把角色改为编辑者后再写入。',
    'This member is a viewer and can only read and pull. Ask an owner to change the role to editor before writing.',
  ],
  device_revoked: [
    '这台设备已被所有者移除或撤销，不能继续同步。请在“备份与同步”页重新配对。',
    'This device was removed or revoked by an owner and can no longer sync. Pair it again in Backup and sync.',
  ],
  sync_not_configured: [
    '本机尚未配置同步。请先在“备份与同步”页设置服务器与设备身份。',
    'Sync is not configured on this device. Set up the server and device identity in Backup and sync.',
  ],
  sync_workspace_not_attached: [
    '当前知识库尚未绑定到同步，请先完成绑定。',
    'This library is not attached to sync yet. Attach it first.',
  ],
  missing_encryption_key: [
    '同步设备缺少加密密钥，请由所有者重新授权密钥后再试。',
    'A sync device is missing its encryption key. Ask an owner to grant the key again.',
  ],
  sync_failed: [
    '同步操作未完成。本地内容已保留，请检查连接和同步设置后重试。',
    'The sync operation did not finish. Local content was kept; check the connection and sync settings, then retry.',
  ],
};
/** Whether retrying the same request can succeed. Mirrors `ApiError::new`. */
const API_RETRYABLE: Record<ApiErrorCode, boolean> = {
  backend_starting: true,
  query_expired: true,
  invalid_input: false,
  invalid_document: false,
  unsupported_api: false,
  instance_mismatch: false,
  app_state_conflict: false,
  revision_conflict: false,
  metadata_conflict: false,
  idempotency_mismatch: false,
  not_found: false,
  object_trashed: false,
  storage_busy: true,
  storage_full: true,
  io_failed: true,
  integrity_failed: false,
  unsupported_database: false,
  migration_failed: false,
  input_unavailable: false,
  output_exists: false,
  internal: false,
};

export function apiErrorMessage(code: ApiErrorCode, language: Language): string {
  const entry = API_MESSAGES[code];
  return entry ? (language === 'en' ? entry[1] : entry[0]) : API_MESSAGES.internal[0];
}

export function apiErrorRetryable(code: ApiErrorCode): boolean {
  return API_RETRYABLE[code] === true;
}

/**
 * Interface text for the free-form `error_code` a background operation or job
 * stores. These codes are not the API enum: they come from the job layer and
 * describe why the work stopped. Anything unknown keeps the raw code visible
 * instead of pretending to know the reason.
 */
const OPERATION_MESSAGES: Record<string, [zh: string, en: string]> = {
  input_unavailable: [
    '文件位置已不可用，请重新选择文件并发起任务。',
    'The file location is unavailable. Select the file again and start a new task.',
  ],
  restored_copy: [
    '这是备份快照中的未完成任务，不会在恢复副本中继续。请重新发起任务。',
    'This task was unfinished when the backup was created. It will not continue in the restored copy. Start it again.',
  ],
  integrity_failed: [
    '备份未通过完整性校验。请保留原文件，并选择另一份完整备份。',
    'The backup failed its integrity check. Keep the original file and choose another complete backup.',
  ],
  storage_full: [
    '目标磁盘空间不足。请释放空间或选择其他位置后重试。',
    'The destination is out of space. Free up space or choose another location.',
  ],
  io_failed: [
    '无法读取或写入文件，请检查文件位置和访问权限。',
    'The file could not be read or written. Check its location and permissions.',
  ],
  storage_busy: [
    '工作区正被其他程序占用，请关闭后重试。',
    'Another program is using the workspace. Close it and try again.',
  ],
  canceled: ['任务已取消，未写入任何结果。', 'The task was canceled and wrote no result.'],
  job_failed: [
    '任务未完成。请检查文件和目标位置，再重新发起。',
    'The task did not finish. Check the file and destination, then start again.',
  ],
  pdf_renderer_unavailable: [
    '未找到本机 Microsoft Edge，无法生成 PDF。请安装或更新 Edge，或改用 DOCX/HTML。',
    'Microsoft Edge is unavailable, so no PDF was generated. Install or update Edge, or export DOCX/HTML instead.',
  ],
  pdf_render_failed: [
    'PDF 渲染失败，未写入目标文件。请更新 Edge，或改用 DOCX/HTML。',
    'PDF rendering failed and no destination file was written. Update Edge, or export DOCX/HTML instead.',
  ],
  pdf_render_timeout: [
    'PDF 渲染超时，未写入目标文件。请缩短文档后重试，或改用 DOCX。',
    'PDF rendering timed out and no destination file was written. Shorten the document or export DOCX instead.',
  ],
  operation_failed: [
    '操作未完成。请检查文件和目标位置，再重新发起。',
    'The action did not finish. Check the file and destination, then start again.',
  ],
  unsupported_database: [
    '这个知识库由更新版本的 spanseek 创建，原数据未被替换，当前版本不会写入或改动它。请更新 spanseek 后重试。',
    'This library was created by a newer spanseek. Your data was left unchanged and this build will not write to it; update spanseek and try again.',
  ],
  migration_failed: [
    '工作区升级检查未通过，请保留原库及升级前快照。',
    'The workspace upgrade check failed. Keep the library and its pre-upgrade snapshot.',
  ],
  invalid_source: [
    '这份资料无法解析。原件已保留，可查看诊断信息后重试。',
    'This material could not be parsed. The original is kept; check diagnostics and retry.',
  ],
  inference_failed: [
    '模型调用未完成。请检查模型配置后重试。',
    'The model call did not finish. Check the model configuration and retry.',
  ],
};

/**
 * Sentences that are only right for one kind of operation.
 *
 * A damaged workspace and a damaged backup share the kernel code
 * integrity_failed, but telling someone to "choose another complete backup"
 * when they tried to open a library sends them to the wrong recovery step.
 */
const WORKSPACE_OPERATION_MESSAGES: Partial<Record<string, [zh: string, en: string]>> = {
  integrity_failed: [
    '这份知识库没有通过完整性检查，文件可能已损坏或被改动。请保留原文件，改用完整备份恢复或打开另一个知识库。',
    'This workspace failed its integrity check; its files may be damaged or changed. Keep the original files and restore a complete backup or open another workspace.',
  ],
};

/** True when the operation stopped because the user asked it to. */
export function operationCanceled(code: string | null | undefined): boolean {
  return code === 'canceled';
}

function workspaceKind(kind: OperationKind | null | undefined): boolean {
  return kind === 'create_workspace' || kind === 'open_workspace';
}

export function operationErrorMessage(
  code: string | null | undefined,
  language: Language,
  kind?: OperationKind | null,
): string {
  const entry =
    (workspaceKind(kind) && code ? WORKSPACE_OPERATION_MESSAGES[code] : undefined) ??
    (code ? OPERATION_MESSAGES[code] : undefined);
  if (entry) return language === 'en' ? entry[1] : entry[0];
  return language === 'en'
    ? 'The task did not finish. Check the file and destination, then start again.'
    : '任务未完成。请检查文件和目标位置，再重新发起。';
}

export function syncFailureReason(error: unknown): string | null {
  if (!error || typeof error !== 'object' || !('details' in error)) return null;
  const details = (error as { details?: { details?: ApiErrorDetails | null } }).details;
  return details?.details?.kind === 'sync_failure' ? details.details.reason : null;
}

/**
 * Interface text for any thrown value, preferring the kernel code over the
 * kernel sentence so the answer is in the language the interface is showing.
 */
export function errorText(error: unknown, language: Language = activeLanguage()): string {
  if (error && typeof error === 'object' && 'details' in error) {
    const details = (
      error as {
        details?: { code?: ApiErrorCode; details?: ApiErrorDetails | null };
      }
    ).details;
    if (details?.details?.kind === 'citation_failure') {
      const entry = CITATION_FAILURE_MESSAGES[details.details.reason];
      if (entry) return language === 'en' ? entry[1] : entry[0];
    }
    if (details?.details?.kind === 'sync_failure') {
      const entry =
        SYNC_FAILURE_MESSAGES[details.details.reason] ?? SYNC_FAILURE_MESSAGES.sync_failed;
      return language === 'en' ? entry[1] : entry[0];
    }
    if (details?.code) return apiErrorMessage(details.code, language);
  }
  if (error instanceof Error && error.message) return error.message;
  return apiErrorMessage('internal', language);
}

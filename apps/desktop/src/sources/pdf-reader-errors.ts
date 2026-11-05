import type { SourceLocator } from '@pkms/contracts';

type Translate = (zh: string, en: string) => string;

export class PdfOriginalReadError extends Error {
  readonly code: 'pdf_range_mismatch' | 'pdf_range_incomplete' | 'pdf_resource_unavailable';
  constructor(code: PdfOriginalReadError['code']) {
    super(code);
    this.name = 'PdfOriginalReadError';
    this.code = code;
  }
}

/** The resource command rejects with an ApiError object; never expose raw paths or ticket URLs. */
export function pdfReaderErrorMessage(error: unknown, t: Translate): string {
  const value =
    error && typeof error === 'object' ? (error as { code?: unknown; name?: unknown }) : null;
  switch (value?.code) {
    case 'io_failed':
      return t(
        '此版本原件读取失败，请检查文件是否可用及访问权限后重试。',
        'The original for this version could not be read. Check file availability and access permissions, then retry.',
      );
    case 'not_found':
    case 'input_unavailable':
    case 'pdf_resource_unavailable':
      return t(
        '此版本原件或读取授权当前不可用，请重试读取此版本。',
        'The original for this version or its read authorization is unavailable. Retry opening this version.',
      );
    case 'integrity_failed':
      return t(
        '此版本原件未通过完整性校验，无法继续读取。请检查原件或从完整备份恢复后重试。',
        'The original for this version failed its integrity check. Check the original or restore it from a complete backup, then retry.',
      );
    case 'instance_mismatch':
      return t(
        '工作区已变化，请返回此资料所属的工作区后重试。',
        'The workspace changed. Return to the workspace containing this document, then retry.',
      );
    case 'storage_busy':
    case 'backend_starting':
      return t(
        '本地存储暂时无法提供原件，请稍后重试。',
        'Local storage cannot provide the original right now. Retry shortly.',
      );
    case 'pdf_range_mismatch':
      return t(
        '原件范围响应不匹配，请重试读取此版本。',
        'The original file range response did not match. Retry opening this version.',
      );
    case 'pdf_range_incomplete':
      return t(
        '原件读取不完整，请重试读取此版本。',
        'The original file range was incomplete. Retry opening this version.',
      );
  }
  if (value?.name === 'PasswordException')
    return t(
      '此 PDF 需要密码，暂时无法打开原件。',
      'This PDF requires a password and cannot be opened yet.',
    );
  if (value?.name === 'InvalidPDFException')
    return t(
      '无法解析此 PDF 原件，请检查文件格式与完整性。',
      'This PDF original could not be parsed. Check its format and integrity.',
    );
  return t(
    '无法读取此 PDF 原件，请重试。',
    'This PDF original could not be read. Retry opening it.',
  );
}

/** An error must never display a quote from a different source version. */
export function pdfErrorExcerpt(versionId: string, location: SourceLocator | null) {
  if (location?.kind !== 'pdf' || location.source_version_id !== versionId) return null;
  return { quote: location.quote, page: location.page_label ?? String(location.page_index + 1) };
}

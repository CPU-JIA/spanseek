import type { WorkspaceAvailability } from '@pkms/contracts';

/** A recent workspace can be opened only when its folder still holds it. */
export function workspaceIsAvailable(availability: WorkspaceAvailability): boolean {
  return availability === 'available';
}

/**
 * One sentence per reason a recent workspace cannot be opened. The kernel
 * distinguishes a folder that vanished from a folder that now carries another
 * library, so the interface must not collapse both into "unavailable".
 *
 * A reason this build does not know is reported as such instead of being guessed.
 */
export function workspaceAvailabilityText(
  availability: WorkspaceAvailability,
  t: (zh: string, en: string) => string,
): string | null {
  switch (availability) {
    case 'available':
      return null;
    case 'missing':
      return t(
        '找不到这个位置或它的 library.sqlite3。',
        'This location or its library.sqlite3 is gone.',
      );
    case 'foreign_library':
      return t(
        '这个位置现在是另一个知识库：打开它会读到别的笔记，请改用那个知识库的入口。',
        'This location now holds a different library; use that library entry instead.',
      );
    case 'unreadable':
      return t(
        '位置存在但无法读取（权限或磁盘未就绪）。',
        'The location exists but cannot be read (permissions or disk).',
      );
    default:
      return t(
        '无法打开这个位置，当前版本不认识这个原因码；请更新应用后重试。',
        'This location cannot be opened and this build does not know the reason; update the app.',
      );
  }
}

/** Short label for the row action; the full sentence stays in the reason line. */
export function workspaceAvailabilityLabel(
  availability: WorkspaceAvailability,
  t: (zh: string, en: string) => string,
): string {
  switch (availability) {
    case 'available':
      return t('打开', 'Open');
    case 'missing':
      return t('位置不可用', 'Unavailable');
    case 'foreign_library':
      return t('另一个知识库', 'Other library');
    case 'unreadable':
      return t('无法读取', 'Unreadable');
    default:
      return t('无法打开', 'Cannot open');
  }
}

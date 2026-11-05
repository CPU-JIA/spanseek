import { message, ServiceError } from '../api';

export function tagErrorMessage(error: unknown, t: (zh: string, en: string) => string): string {
  if (error instanceof ServiceError) {
    if (error.details.code === 'query_expired')
      return t(
        '标签的使用范围已变化。请重新查看影响范围，再确认删除。',
        'Tag usage has changed. Review the affected items again before deleting.',
      );
    if (error.details.code === 'metadata_conflict')
      return t(
        '标签或关联内容已在别处修改。请重新读取，再应用你的修改。',
        'The tag or linked content changed elsewhere. Reload it before applying your edits.',
      );
    const detail = error.details.details;
    if (detail?.kind === 'tag_failure') {
      if (detail.reason === 'duplicate_name')
        return t(
          '已有同名标签，请使用其他名称。',
          'A tag with this name already exists. Choose another name.',
        );
      if (detail.reason === 'view_in_use')
        return t(
          '保存视图仍在使用这个标签。请先修改这些视图的筛选条件。',
          'Saved views still use this tag. Update their filters first.',
        );
      if (detail.reason === 'invalid_description')
        return t(
          '标签说明最多 4000 字，不能包含空字符。',
          'Use at most 4,000 characters without null characters in the description.',
        );
    }
  }
  return message(error);
}

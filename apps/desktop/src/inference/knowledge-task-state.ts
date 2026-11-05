import type {
  ChatRunStatus,
  KnowledgeTaskTemplateRef,
  SearchEvidence,
  KnowledgeTaskMaterialCoverage,
} from '@pkms/contracts';

export type TaskDraft = {
  title: string;
  instructions: string;
  materials: SearchEvidence[];
  template: KnowledgeTaskTemplateRef | null;
  templateName: string;
  retryOf: string | null;
  materialWarnings?: KnowledgeTaskMaterialCoverage[];
  materialWarningsAccepted?: boolean;
};
export const emptyTaskDraft = (): TaskDraft => ({
  title: '',
  instructions: '',
  materials: [],
  template: null,
  templateName: '',
  retryOf: null,
});

/** Local drafts are untrusted input; Rust revalidates material snapshots on submission. */
export function parseTaskDraft(raw: string): TaskDraft {
  const value = JSON.parse(raw) as TaskDraft;
  if (
    !value ||
    typeof value.title !== 'string' ||
    typeof value.instructions !== 'string' ||
    typeof value.templateName !== 'string' ||
    !Array.isArray(value.materials) ||
    value.materials.length > 32 ||
    (value.retryOf !== null && typeof value.retryOf !== 'string') ||
    (value.materialWarnings !== undefined &&
      (!Array.isArray(value.materialWarnings) ||
        value.materialWarnings.length > 100 ||
        value.materialWarnings.some(
          (item) =>
            !item ||
            typeof item.object_id !== 'string' ||
            typeof item.title !== 'string' ||
            ![
              'complete',
              'limited',
              'empty',
              'missing',
              'trashed',
              'search_disabled',
              'unparsed',
              'changed',
              'unsupported',
            ].includes(item.status) ||
            (item.total_characters !== null &&
              (!Number.isInteger(item.total_characters) || item.total_characters < 0)) ||
            !Number.isInteger(item.included_characters) ||
            item.included_characters < 0 ||
            !Number.isInteger(item.included_fragments) ||
            item.included_fragments < 0,
        ))) ||
    (value.materialWarningsAccepted !== undefined &&
      typeof value.materialWarningsAccepted !== 'boolean') ||
    (value.template !== null &&
      (!value.template ||
        typeof value.template.id !== 'string' ||
        !Number.isInteger(value.template.row_version) ||
        value.template.row_version < 1)) ||
    value.materials.some(
      (item) =>
        !item ||
        typeof item.id !== 'string' ||
        typeof item.text !== 'string' ||
        typeof item.title !== 'string' ||
        !item.target ||
        typeof item.target.object_id !== 'string',
    )
  ) {
    throw new Error('Invalid saved knowledge task draft');
  }
  return value;
}

export function mergeTaskMaterials(
  draft: TaskDraft,
  materials: SearchEvidence[],
  replacedIds: readonly string[],
  warnings: KnowledgeTaskMaterialCoverage[],
): TaskDraft {
  const replaced = new Set(replacedIds);
  const objectId = (e: SearchEvidence) => e.annotation?.annotation_id ?? e.target.object_id;
  const emitted = new Set<string>();
  const combined = draft.materials.flatMap((e) => {
    const id = objectId(e);
    if (!replaced.has(id)) return [e];
    if (emitted.has(id)) return [];
    emitted.add(id);
    return materials.filter((item) => objectId(item) === id);
  });
  combined.push(...materials.filter((e) => !emitted.has(objectId(e))));
  return {
    ...draft,
    materials: combined,
    materialWarnings: [
      ...(draft.materialWarnings ?? []).filter((e) => !replaced.has(e.object_id)),
      ...warnings,
    ],
    materialWarningsAccepted: false,
  };
}

export function materialCoverageText(
  item: KnowledgeTaskMaterialCoverage,
  t: (zh: string, en: string) => string,
) {
  if (item.status === 'limited' && item.total_characters !== null)
    return t(
      `已载入 ${item.included_characters} / ${item.total_characters} 个字符，请核对片段`,
      `${item.included_characters} / ${item.total_characters} characters loaded; review the excerpts`,
    );
  const names: Record<string, string> = {
    complete: t('已完整载入可用正文', 'All available text loaded'),
    limited: t('超过本次容量，仅载入部分内容', 'Only part of the content fits this task'),
    empty: t('没有可用正文', 'No usable text'),
    missing: t('条目已不可用', 'Item unavailable'),
    trashed: t('条目在回收站', 'Item is in trash'),
    search_disabled: t('已停用检索，未读取正文', 'Search is disabled; no text was read'),
    unparsed: t('尚未解析出正文', 'Text has not been parsed'),
    changed: t(
      '版本已变化，未替换成新版本',
      'The revision changed; no newer version was substituted',
    ),
    unsupported: t('请直接选择资料、笔记或摘录', 'Select sources, notes or excerpts directly'),
  };
  return names[item.status] ?? t('需要重新核对材料', 'Review this material');
}

export function taskStatus(status: ChatRunStatus, t: (zh: string, en: string) => string) {
  const labels: Record<ChatRunStatus, string> = {
    queued: t('等待生成', 'Queued'),
    running: t('正在生成', 'Generating'),
    completed: t('生成完成', 'Generated'),
    canceled: t('已取消', 'Canceled'),
    interrupted: t('已中断', 'Interrupted'),
    timed_out: t('已超时', 'Timed out'),
    failed: t('生成失败', 'Failed'),
    truncated: t('内容未生成完整', 'Incomplete output'),
    refused: t('模型未接受请求', 'Request declined'),
    model_unconfigured: t('未配置模型', 'Model not configured'),
  };
  return labels[status];
}

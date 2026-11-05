import { inferenceErrorMessage } from '../inference/inference-errors.ts';

type Translate = (zh: string, en: string) => string;

export type KnowledgeSearchNotice = {
  tone: 'status' | 'alert';
  text: string;
};

export function knowledgeSearchNotices(
  input: {
    lexicalAvailable: boolean;
    semanticAvailable: boolean;
    unavailableReason: string | null;
    idCount: number;
  },
  t: Translate,
  limit = 100,
): KnowledgeSearchNotice[] {
  const notices: KnowledgeSearchNotice[] = [];
  if (!input.semanticAvailable) {
    notices.push({
      tone: 'status',
      text:
        inferenceErrorMessage(input.unavailableReason ?? 'index_unavailable', t) +
        ' · ' +
        t('当前显示关键词命中的对象。', 'Keyword-matched objects are shown.'),
    });
  }
  if (!input.lexicalAvailable) {
    notices.push({
      tone: 'alert',
      text: t(
        '关键词检索也不可用，本次模糊查找没有可用结果。',
        'Keyword search is also unavailable, so this fuzzy search has no usable results.',
      ),
    });
  }
  if (input.idCount >= limit) {
    notices.push({
      tone: 'status',
      text: t(
        `模糊结果最多返回前 ${limit} 个对象；可用分类或标签缩小范围。`,
        `Fuzzy search returns at most the first ${limit} objects. Narrow the category or tags to see more.`,
      ),
    });
  }
  return notices;
}

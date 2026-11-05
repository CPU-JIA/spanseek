/**
 * The refusal a reader sees when the library holds no evidence for a question.
 *
 * FA4-07 requires a fixed, predictable message that never invents content, so
 * this copy is authored here rather than taken from any textbook, and it is
 * pinned by tests/refusal-copy.test.mjs. The stored answer text stays the
 * machine marker (pkms_core::INSUFFICIENT_EVIDENCE_MARKER) so a refusal is
 * machine-detectable; this copy is what a person reads, and the marker is never
 * rendered.
 */
export const REFUSAL_MARKER = '[[PKMS_INSUFFICIENT_EVIDENCE]]';

export type RefusalCopy = {
  /** Short label for status lines, lists and notifications. */
  label: string;
  /** What happened and why no conclusion was given. */
  body: string;
  /** Actionable next steps, in the order a reader should try them. */
  steps: string[];
};

const CHINESE: RefusalCopy = {
  label: '未找到可支撑结论的资料',
  body: '已在你授权的范围内完成检索，但库内资料不足以支撑一个可追溯的回答。为避免编造，本次不给出结论，也不引用任何未经核对的来源。',
  steps: [
    '导入或补充相关资料后重新提问。',
    '如果当前限定了范围或标签，扩大范围后再检索一次。',
    '换一种问法，或改问资料中明确写到的内容。',
  ],
};

const ENGLISH: RefusalCopy = {
  label: 'No source in this library supports an answer',
  body: 'The search covered your authorised scope, but the library does not hold enough evidence for a traceable answer. Nothing was invented: no conclusion is given, and no unchecked source is cited.',
  steps: [
    'Import or add the relevant source, then ask again.',
    'If a scope or tag is applied, widen it and search once more.',
    'Rephrase the question, or ask about something the sources state explicitly.',
  ],
};

export function refusalCopy(english: boolean): RefusalCopy {
  return english ? ENGLISH : CHINESE;
}

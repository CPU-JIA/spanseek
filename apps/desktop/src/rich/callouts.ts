export const CALLOUT_TYPES = [
  { type: 'note', label: '笔记', en: 'Note', color: '#2563EB', icon: 'pencil' },
  { type: 'abstract', label: '摘要', en: 'Abstract', color: '#0891B2', icon: 'list' },
  { type: 'info', label: '信息', en: 'Info', color: '#2563EB', icon: 'info' },
  { type: 'todo', label: '待办', en: 'Todo', color: '#2563EB', icon: 'circle-check' },
  { type: 'tip', label: '提示', en: 'Tip', color: '#0891B2', icon: 'lightbulb' },
  { type: 'success', label: '成功', en: 'Success', color: '#15803D', icon: 'check' },
  { type: 'question', label: '问题', en: 'Question', color: '#B45309', icon: 'circle-help' },
  { type: 'warning', label: '注意', en: 'Warning', color: '#B45309', icon: 'triangle-alert' },
  { type: 'failure', label: '失败', en: 'Failure', color: '#DC2626', icon: 'x' },
  { type: 'danger', label: '危险', en: 'Danger', color: '#DC2626', icon: 'zap' },
  { type: 'bug', label: '缺陷', en: 'Bug', color: '#DC2626', icon: 'bug' },
  { type: 'example', label: '示例', en: 'Example', color: '#7C3AED', icon: 'flask-conical' },
  { type: 'quote', label: '引文', en: 'Quote', color: '#64748B', icon: 'quote' },
] as const;
export const CALLOUT_ALIASES: Readonly<Record<string, string>> = Object.freeze({
  summary: 'abstract',
  tldr: 'abstract',
  hint: 'tip',
  important: 'tip',
  check: 'success',
  done: 'success',
  help: 'question',
  faq: 'question',
  caution: 'warning',
  attention: 'warning',
  fail: 'failure',
  missing: 'failure',
  error: 'danger',
  cite: 'quote',
});
export function getCallout(type: string | null | undefined) {
  const name = (type ?? 'note').toLowerCase();
  const canonical = Object.hasOwn(CALLOUT_ALIASES, name) ? CALLOUT_ALIASES[name] : name;
  return CALLOUT_TYPES.find((value) => value.type === canonical) ?? CALLOUT_TYPES[0];
}

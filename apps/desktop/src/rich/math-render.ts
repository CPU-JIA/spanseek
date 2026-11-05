import katex from 'katex';
import { MATH_LIMITS } from './render-policy.ts';

/** Source remains the stored value; this HTML is disposable and never written back. */
export function renderMath(source: string, display: boolean): string {
  if (source.length > MATH_LIMITS.characters) throw new Error('公式超出预览限额，请拆分公式。');
  let denied = false;
  const html = katex.renderToString(source, {
    displayMode: display,
    output: 'htmlAndMathml',
    throwOnError: true,
    trust: () => {
      denied = true;
      return false;
    },
    maxExpand: MATH_LIMITS.maxExpand,
    maxSize: MATH_LIMITS.maxSize,
    macros: {},
  });
  if (denied) throw new Error('公式中的链接、资源或 HTML 命令未启用，源码仍保留。');
  return html;
}

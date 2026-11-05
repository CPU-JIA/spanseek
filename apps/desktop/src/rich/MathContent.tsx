import { useMemo } from 'react';
import { useI18n } from '../i18n';
import { renderMath } from './math-render';
import 'katex/dist/katex.min.css';
import './rich-content.css';

export function MathContent({ source, display }: { source: string; display: boolean }) {
  const { t } = useI18n();
  const result = useMemo(() => {
    try {
      return { html: renderMath(source, display), error: null };
    } catch (cause) {
      return {
        html: '',
        error: cause instanceof Error ? cause.message : '公式无法渲染，源码仍保留。',
      };
    }
  }, [source, display]);
  const Tag = display ? 'div' : 'span';
  return result.error ? (
    <Tag className="rich-math rich-format-error" title={result.error}>
      <code>
        {display ? '$$' : '$'}
        {source}
        {display ? '$$' : '$'}
      </code>
      <span className="rich-caption">{t('公式未能渲染', 'The formula could not be rendered')}</span>
    </Tag>
  ) : (
    <Tag
      className={display ? 'rich-math rich-math-block' : 'rich-math'}
      dangerouslySetInnerHTML={{ __html: result.html }}
    />
  );
}

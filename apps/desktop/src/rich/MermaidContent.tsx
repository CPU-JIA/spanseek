import { useEffect, useRef, useState } from 'react';
import { useI18n } from '../i18n';
import { renderMermaid } from './mermaid-render';
import './rich-content.css';

export function MermaidContent({ source }: { source: string }) {
  const { t } = useI18n();
  const host = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(false);
  const [state, setState] = useState<{ source: string; svg?: string; error?: string } | null>(null);
  useEffect(() => {
    if (!host.current) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setNear(true);
          observer.disconnect();
        }
      },
      { rootMargin: '200px' },
    );
    observer.observe(host.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (!near) return;
    const abort = new AbortController();
    void Promise.resolve()
      .then(() => renderMermaid(source, abort.signal))
      .then((svg) => {
        if (!abort.signal.aborted) setState({ source, svg });
      })
      .catch((cause) => {
        if (!abort.signal.aborted)
          setState({ source, error: cause instanceof Error ? cause.message : '图表无法渲染。' });
      });
    return () => abort.abort();
  }, [source, near]);
  const current = state?.source === source ? state : null;
  return (
    <div ref={host} className="rich-mermaid">
      {current?.svg ? (
        <div className="rich-mermaid-diagram" dangerouslySetInnerHTML={{ __html: current.svg }} />
      ) : (
        <p
          className={current?.error ? 'inline-error' : 'rich-caption'}
          role={current?.error ? 'alert' : 'status'}
        >
          {current?.error ?? t('图表正在准备…', 'Preparing diagram…')}
        </p>
      )}
      <details className="rich-source" open={Boolean(current?.error)}>
        <summary>{t('Mermaid 源码', 'Mermaid source')}</summary>
        <pre>
          <code>{source}</code>
        </pre>
      </details>
    </div>
  );
}

import { renderMath } from '../rich/math-render';
import { renderMermaid } from '../rich/mermaid-render';
import './style.css';

const selector = '[data-pkms-math="inline"],[data-pkms-math="block"],pre[data-pkms-mermaid]';
const sourceGroups = new WeakMap<HTMLElement, HTMLElement>();

function sourceDetails(source: HTMLElement, label: string): HTMLDetailsElement {
  const details = document.createElement('details');
  details.className = 'pkms-rich-source';
  const summary = document.createElement('summary');
  summary.textContent = label;
  const text = document.createElement('div');
  text.className = 'pkms-rich-source-text';
  text.append(source);
  details.append(summary, text);
  return details;
}

function inlineSourceGroup(source: HTMLElement): HTMLElement {
  const paragraph = source.closest<HTMLElement>('p,h1,h2,h3,h4,h5,h6');
  const container =
    paragraph ??
    source.closest<HTMLElement>('td,th,li,blockquote,section,article,main,div,body') ??
    document.body;
  const existing = sourceGroups.get(container);
  if (existing) return existing;
  const group = document.createElement('div');
  group.className = 'pkms-rich-inline-sources';
  if (paragraph) paragraph.after(group);
  else container.append(group);
  sourceGroups.set(container, group);
  return group;
}

function showPreview(
  source: HTMLElement,
  html: string,
  kind: 'inline' | 'block' | 'mermaid',
): void {
  // Inline source controls live after the containing paragraph, never inside it.
  const sourceGroup = kind === 'inline' ? inlineSourceGroup(source) : undefined;
  const preview = document.createElement(kind === 'inline' ? 'span' : 'div');
  preview.className = `pkms-rich-preview pkms-rich-${kind}`;
  preview.dataset.pkmsRichPreview = kind;
  // Only the existing KaTeX/validated Mermaid adapters supply this HTML.
  preview.innerHTML = html;
  if (source.id) {
    preview.id = source.id;
    source.removeAttribute('id');
  }
  source.replaceWith(preview);
  source.dataset.pkmsRichState = 'rendered';
  const details = sourceDetails(source, kind === 'mermaid' ? '图表源码' : '公式源码');
  if (sourceGroup) sourceGroup.append(details);
  else preview.after(details);
}

function showFailure(
  source: HTMLElement,
  kind: 'inline' | 'block' | 'mermaid',
  error: unknown,
): void {
  source.dataset.pkmsRichState = 'error';
  const notice = document.createElement(kind === 'inline' ? 'span' : 'p');
  notice.className = 'pkms-rich-error';
  notice.setAttribute('role', 'status');
  const reason = error instanceof Error ? error.message : '渲染失败。';
  // Parse errors may contain user source. Always use textContent, never HTML.
  notice.textContent = `${kind === 'mermaid' ? '图表' : '公式'}未能渲染：${reason}`;
  source.after(notice);
}

async function renderExport(): Promise<void> {
  const nodes = Array.from(document.querySelectorAll<HTMLElement>(selector)).filter(
    (element) => !element.dataset.pkmsRichState,
  );
  if (nodes.length === 0) return;
  const abort = new AbortController();
  window.addEventListener('pagehide', () => abort.abort(), { once: true });
  document.documentElement.dataset.pkmsRichExport = 'rendering';

  for (const [index, source] of nodes.entries()) {
    if (abort.signal.aborted) return;
    const kind = source.matches('pre[data-pkms-mermaid]')
      ? 'mermaid'
      : source.dataset.pkmsMath === 'inline'
        ? 'inline'
        : 'block';
    const original = source.textContent ?? '';
    source.dataset.pkmsRichState = 'rendering';
    try {
      const html =
        kind === 'mermaid'
          ? await renderMermaid(original, abort.signal)
          : renderMath(original, kind === 'block');
      if (abort.signal.aborted) return;
      showPreview(source, html, kind);
    } catch (error) {
      if (abort.signal.aborted) return;
      showFailure(source, kind, error);
    }
    // Avoid monopolizing the page when an export contains many short formulae.
    if (index % 16 === 15) await new Promise<void>((resolve) => setTimeout(resolve, 0));
  }
  document.documentElement.dataset.pkmsRichExport = 'complete';
}

function start(): void {
  void renderExport();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', start, { once: true });
} else {
  start();
}

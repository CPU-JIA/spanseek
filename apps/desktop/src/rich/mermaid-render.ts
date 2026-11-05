import { assertMermaidSource, assertMermaidSvg, MERMAID_CONFIG } from './render-policy';

let sequence = 0;
let queue: Promise<unknown> = Promise.resolve();

/** Lazy, serialized use of Mermaid's shared configuration. Never bind source callbacks. */
export function renderMermaid(source: string, signal?: AbortSignal): Promise<string> {
  assertMermaidSource(source);
  const pending = queue.then(async () => {
    signal?.throwIfAborted();
    const { default: mermaid } = await import('mermaid');
    signal?.throwIfAborted();
    mermaid.initialize({ ...MERMAID_CONFIG, secure: [...MERMAID_CONFIG.secure] });
    const host = document.createElement('div');
    host.setAttribute('aria-hidden', 'true');
    Object.assign(host.style, {
      position: 'fixed',
      left: '-100000px',
      top: '0',
      width: '1200px',
      visibility: 'hidden',
    });
    document.body.append(host);
    try {
      const { svg } = await mermaid.render(`pkms-mermaid-${++sequence}`, source, host);
      signal?.throwIfAborted();
      assertMermaidSvg(svg);
      return svg;
    } finally {
      host.remove();
    }
  });
  queue = pending.catch(() => undefined);
  return pending;
}

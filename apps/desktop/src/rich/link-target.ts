export type OrdinaryLinkTarget =
  { kind: 'external'; href: string } | { kind: 'anchor'; href: string; anchor: string };

/** Match the stored link protocol boundary; never delegate arbitrary protocols to the OS. */
export function ordinaryLinkTarget(input: string): OrdinaryLinkTarget {
  const href = input.trim();
  if (!href) throw new Error('链接地址为空，请先编辑链接。');
  if (new TextEncoder().encode(href).length > 8192 || /[\u0000-\u0020\u007f-\u009f]/.test(href))
    throw new Error('链接过长或含有空格、控制字符，请先编辑链接。');
  if (href.startsWith('#')) {
    let anchor: string;
    try {
      anchor = decodeURIComponent(href.slice(1));
    } catch {
      throw new Error('链接锚点编码无效，请先编辑链接。');
    }
    if (!anchor) throw new Error('链接锚点为空。');
    return { kind: 'anchor', href, anchor };
  }
  if (!/^(?:https?:\/\/|mailto:)/i.test(href))
    throw new Error('此链接协议不受支持；仅可打开 https://、http:// 或 mailto: 链接。');
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    throw new Error('链接地址无效，请先编辑链接。');
  }
  if (
    (url.protocol === 'mailto:' && !url.pathname) ||
    (url.protocol !== 'mailto:' && !url.hostname)
  )
    throw new Error('链接缺少网站或邮箱地址，请先编辑链接。');
  return { kind: 'external', href };
}

/** #targets use the existing stable internal-link node; ordinary marks cannot persist anchors. */
export function storedOrdinaryLink(input: string): string {
  const target = ordinaryLinkTarget(input);
  if (target.kind === 'anchor')
    throw new Error('本文标题或段落请使用“内部链接”，输入 #标题 或 #^块标识。');
  return target.href;
}

export function localLinkAnchor(root: HTMLElement, anchor: string): HTMLElement | null {
  const scope = root.closest<HTMLElement>('.rich-document-content, .note-prose') ?? root;
  const elements = Array.from(
    scope.querySelectorAll<HTMLElement>('[id], [data-id], [data-block-id], h1, h2, h3, h4, h5, h6'),
  ).filter((element) => element.closest('.rich-document-content, .note-prose') === scope);
  const block = anchor.startsWith('^') ? anchor.slice(1) : anchor;
  return (
    elements.find(
      (element) =>
        element.id === anchor || element.dataset.id === anchor || element.dataset.blockId === block,
    ) ??
    elements.find(
      (element) => /^H[1-6]$/.test(element.tagName) && element.textContent?.trim() === anchor,
    ) ??
    null
  );
}

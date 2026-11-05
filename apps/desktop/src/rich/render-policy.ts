export const MATH_LIMITS = Object.freeze({ characters: 12000, maxExpand: 500, maxSize: 20 });
export const MERMAID_LIMITS = Object.freeze({ characters: 20000, lines: 500, edges: 200 });
export const MERMAID_CONFIG = Object.freeze({
  startOnLoad: false,
  securityLevel: 'strict' as const,
  htmlLabels: false,
  flowchart: { htmlLabels: false },
  maxTextSize: MERMAID_LIMITS.characters,
  maxEdges: MERMAID_LIMITS.edges,
  suppressErrorRendering: true,
  fontFamily: 'system-ui, sans-serif',
  secure: [
    'secure',
    'securityLevel',
    'startOnLoad',
    'maxTextSize',
    'maxEdges',
    'suppressErrorRendering',
    'htmlLabels',
    'flowchart',
    'dompurifyConfig',
    'themeCSS',
    'fontFamily',
    'altFontFamily',
  ],
});

function decoded(value: string): string {
  return value.replace(/&#(x[\da-f]+|\d+);?|&(colon|sol|tab|newline);/gi, (all, numeric, named) => {
    if (numeric) {
      const code =
        numeric[0].toLowerCase() === 'x' ? parseInt(numeric.slice(1), 16) : parseInt(numeric, 10);
      return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : all;
    }
    return ({ colon: ':', sol: '/', tab: '\t', newline: '\n' } as Record<string, string>)[
      named.toLowerCase()
    ];
  });
}

/** Check before Mermaid runs: SVG cleanup cannot undo an image request made during layout. */
export function assertMermaidSource(source: string): void {
  if (!source.trim()) throw new Error('图表源码为空。');
  if (
    source.length > MERMAID_LIMITS.characters ||
    source.split(/\r?\n/).length > MERMAID_LIMITS.lines
  )
    throw new Error('图表超出预览限额，请拆分后再渲染。');
  const text = decoded(source);
  if (/%%\s*\{|^\s*---(?:\r?\n|$)/m.test(text))
    throw new Error('图表内不能覆盖渲染配置；源码已保留。');
  if (
    /(?:^|[;\r\n])\s*click\b/i.test(text) ||
    /\b(?:href|src|img|image|icon|securityLevel|themeCSS|fontFamily|htmlLabels|config)["']?\s*:/i.test(
      text,
    )
  )
    throw new Error('图表的点击脚本、资源和配置指令未启用；源码已保留。');
  if (
    /(?:https?|javascript|vbscript|data|file|ftp|blob)\s*:|\/\/|\burl\s*\(|@import|@font-face|\bexpression\s*\(/i.test(
      text,
    )
  )
    throw new Error('图表不能加载外部地址或样式资源；源码已保留。');
  if (
    /<\s*\/?\s*(?:script|iframe|object|embed|img|image|foreignObject|a|style|link|svg|audio|video)\b|\bon[a-z]+\s*=/i.test(
      text,
    )
  )
    throw new Error('图表包含不支持的可执行或外部内容；源码已保留。');
}

/** A conservative, DOM-independent boundary for the generated SVG, shared with export. */
export function assertMermaidSvg(svg: string): void {
  const text = decoded(svg);
  if (
    !/^\s*<svg\b/i.test(text) ||
    /<!DOCTYPE|<!ENTITY|<\s*(?:script|foreignObject|image|img|iframe|object|embed|a|link|audio|video)\b|\bon[a-z]+\s*=/i.test(
      text,
    )
  )
    throw new Error('图表输出包含不支持的内容，已保留源码。');
  for (const attribute of text.matchAll(/\s(?:xlink:)?href\s*=\s*(["'])(.*?)\1/gi)) {
    if (!/^#[\w:.-]+$/.test(attribute[2])) throw new Error('图表输出包含外部链接，已保留源码。');
  }
  for (const resource of text.matchAll(/url\s*\(([^)]*)\)/gi)) {
    if (!/^\s*["']?#[\w:.-]+["']?\s*$/.test(resource[1]))
      throw new Error('图表输出包含外部资源，已保留源码。');
  }
  if (/@import|@font-face|\bexpression\s*\(|javascript\s*:|vbscript\s*:/i.test(text))
    throw new Error('图表输出包含不支持的样式，已保留源码。');
}

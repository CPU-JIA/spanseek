import type { DocumentMark } from '@pkms/contracts';
import { parseFragment, type DefaultTreeAdapterMap } from 'parse5';

type HtmlElement = DefaultTreeAdapterMap['element'];
type HtmlNode = DefaultTreeAdapterMap['node'];
export type HtmlFormatToken =
  | { kind: 'open'; name: string; marks: DocumentMark[] }
  | { kind: 'close'; name: string }
  | { kind: 'break' };
export type HtmlText = { text: string; marks: DocumentMark[] } | { hardBreak: true };

const simpleMarks = {
  strong: 'bold',
  b: 'bold',
  em: 'italic',
  i: 'italic',
  s: 'strike',
  del: 'strike',
  u: 'underline',
  sup: 'superscript',
  sub: 'subscript',
  code: 'code',
} as const;
const colorNames: Record<string, string> = {
  black: '#000000',
  silver: '#C0C0C0',
  gray: '#808080',
  grey: '#808080',
  white: '#FFFFFF',
  maroon: '#800000',
  red: '#FF0000',
  purple: '#800080',
  fuchsia: '#FF00FF',
  green: '#008000',
  lime: '#00FF00',
  olive: '#808000',
  yellow: '#FFFF00',
  navy: '#000080',
  blue: '#0000FF',
  teal: '#008080',
  aqua: '#00FFFF',
  orange: '#FFA500',
  pink: '#FFC0CB',
  hotpink: '#FF69B4',
  gold: '#FFD700',
  brown: '#A52A2A',
  cyan: '#00FFFF',
  magenta: '#FF00FF',
  violet: '#EE82EE',
  indigo: '#4B0082',
  rebeccapurple: '#663399',
  transparent: '',
};

/** Converts the supported CSS color forms into the persisted six-digit form.
 * CSS functions, variables, alpha and arbitrary declarations are never retained.
 */
export function markdownColor(value: string): string | null {
  const color = value.trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(color)) return color.toUpperCase();
  if (/^#[0-9a-f]{3}$/.test(color))
    return `#${[...color.slice(1)].map((digit) => digit + digit).join('')}`.toUpperCase();
  const rgb = color.match(/^rgb\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*\)$/);
  if (rgb) {
    const channels = rgb.slice(1).map(Number);
    if (channels.every((channel) => channel <= 255))
      return `#${channels.map((channel) => channel.toString(16).padStart(2, '0')).join('')}`.toUpperCase();
  }
  return colorNames[color] || null;
}

export function safeMarkdownLink(value: string): boolean {
  return (
    /^(https?:\/\/|mailto:)/i.test(value) &&
    !/[\x00-\x1f\x7f-\x9f]/.test(value) &&
    new TextEncoder().encode(value).length <= 8192
  );
}

/** Decode CommonMark escapes/entities as plain characters, never as elements. */
export function markdownPlainString(source: string): string {
  const escaped = source.replace(
    /\\([!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~])/g,
    (_match, character: string) => `&#${character.codePointAt(0)};`,
  );
  const parsed = parseFragment(escaped.replace(/</g, '&lt;'));
  return parsed.childNodes.map((node) => ('value' in node ? node.value : '')).join('');
}

function elementMarks(element: HtmlElement): DocumentMark[] | null {
  if (element.namespaceURI !== 'http://www.w3.org/1999/xhtml') return null;
  const { tagName: name, attrs } = element;
  if (Object.hasOwn(simpleMarks, name)) {
    if (attrs.length) return null;
    return [{ type: simpleMarks[name as keyof typeof simpleMarks] }];
  }
  if (name === 'a') {
    if (attrs.some((attr) => !['href', 'title', 'rel', 'target', 'class'].includes(attr.name)))
      return null;
    const href = attrs.find((attr) => attr.name === 'href')?.value;
    if (!href || !safeMarkdownLink(href)) return null;
    const target = attrs.find((attr) => attr.name === 'target')?.value;
    const rel = attrs.find((attr) => attr.name === 'rel')?.value;
    if (target && target !== '_blank' && target !== '_self') return null;
    const className = attrs.find((attr) => attr.name === 'class')?.value;
    if (className) return null;
    if (rel && /[\x00-\x1f\x7f-\x9f]/.test(rel)) return null;
    const title = attrs.find((attr) => attr.name === 'title')?.value;
    return [
      {
        type: 'link',
        attrs: {
          href,
          ...(title !== undefined ? { title } : {}),
          ...(target !== undefined ? { target } : {}),
          ...(rel !== undefined ? { rel } : {}),
          ...(className !== undefined ? { class: className } : {}),
        },
      },
    ];
  }
  if (name !== 'mark' && name !== 'span') return null;
  if (attrs.some((attr) => attr.name !== 'style')) return null;
  const style = attrs.find((attr) => attr.name === 'style')?.value;
  if (!style) return name === 'mark' ? [{ type: 'highlight' }] : null;
  const colors = new Map<string, string>();
  for (const declaration of style.split(';')) {
    if (!declaration.trim()) continue;
    const match = declaration.trim().match(/^(color|background-color)\s*:\s*(.+)$/i);
    if (!match) return null;
    const property = match[1].toLowerCase();
    const color = markdownColor(match[2]);
    if (!color || colors.has(property)) return null;
    colors.set(property, color);
  }
  if (!colors.size) return null;
  const result: DocumentMark[] = [];
  const background = colors.get('background-color');
  const foreground = colors.get('color');
  if (name === 'mark' || background)
    result.push({ type: 'highlight', ...(background ? { attrs: { color: background } } : {}) });
  if (foreground) result.push({ type: 'textStyle', attrs: { color: foreground } });
  return result;
}

export function combineMarkdownMarks(
  current: DocumentMark[],
  incoming: DocumentMark[],
): DocumentMark[] | null {
  const result = [...current];
  for (const mark of incoming) {
    if (
      (mark.type === 'superscript' && result.some((item) => item.type === 'subscript')) ||
      (mark.type === 'subscript' && result.some((item) => item.type === 'superscript'))
    )
      return null;
    const index = result.findIndex((item) => item.type === mark.type);
    if (index < 0) result.push(mark);
    else result[index] = mark;
  }
  return result.length > 1 && result.some((mark) => mark.type === 'code') ? null : result;
}

function fragment(source: string) {
  let invalid = false;
  const value = parseFragment(source, {
    onParseError: () => {
      invalid = true;
    },
  });
  return invalid ? null : value;
}

/** Accepts exactly one format tag. Unknown attributes and malformed HTML stay source. */
export function htmlFormatToken(source: string): HtmlFormatToken | null {
  const closing = source.match(/^<\/([a-z][a-z0-9]*)\s*>$/i);
  if (closing) {
    const name = closing[1].toLowerCase();
    return Object.hasOwn(simpleMarks, name) || ['a', 'mark', 'span'].includes(name)
      ? { kind: 'close', name }
      : null;
  }
  if (!/^<[a-z][a-z0-9]*(?:\s[^<>]*|\s*\/?)>$/i.test(source)) return null;
  const parsed = fragment(source);
  if (!parsed || parsed.childNodes.length !== 1) return null;
  const element = parsed.childNodes[0];
  if (!('tagName' in element)) return null;
  if (element.tagName === 'br' && !element.attrs.length) return { kind: 'break' };
  const marks = elementMarks(element);
  return marks ? { kind: 'open', name: element.tagName, marks } : null;
}

/** Worker-safe parsing of a complete inline HTML block; no HTML is ever rendered. */
export function htmlText(source: string): HtmlText[] | null {
  const parsed = fragment(source);
  if (!parsed) return null;
  const output: HtmlText[] = [];
  function visit(node: HtmlNode, marks: DocumentMark[], depth: number): boolean {
    if (depth > 28) return false;
    if (node.nodeName === '#text' && 'value' in node) {
      if (node.value) output.push({ text: node.value, marks });
      return true;
    }
    if (!('tagName' in node)) return false;
    if (node.tagName === 'br' && !node.attrs.length) {
      output.push({ hardBreak: true });
      return true;
    }
    const own = elementMarks(node);
    const next = own && combineMarkdownMarks(marks, own);
    return !!next && node.childNodes.every((child) => visit(child, next, depth + 1));
  }
  return parsed.childNodes.every((node) => visit(node, [], 0)) ? output : null;
}

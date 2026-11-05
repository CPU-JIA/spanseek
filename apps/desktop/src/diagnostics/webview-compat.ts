/**
 * Which WebView2 the interface actually needs, and what an older one silently loses.
 *
 * Tauri's downloadBootstrapper ensures a runtime exists. The application reports
 * unsupported features separately using these thresholds, each tied to the file
 * that uses the feature:
 *
 * - `123.0.0.0` `field-sizing` — `inference/chat.css` grows the composer with its text.
 * - `111.0.0.0` `color-mix()` — the focus glow in `inference/inference.css`, `chat.css`.
 * - `105.0.0.0` `:has()` and `@container` — selected-state and narrow-window rules in
 *   `styles.css`, `review/review.css`, `organization/organization-pages.css`.
 * - `94.0.0.0` ES2022 — `vite.config.ts` `build.target`; below it the scripts themselves
 *   stop, which is why this is the floor rather than a cosmetic loss.
 *
 * A WebView2 version tracks the Edge/Chromium major, so `111.0.1661.44` is Chromium 111.
 */

import type { Language } from '../language.ts';

type Bilingual = readonly [zh: string, en: string];

export type WebView2Threshold = {
  /** First runtime version with the feature, dotted, as the runtime reports it. */
  version: string;
  /** The feature as it is written in the stylesheet or the build config. */
  feature: string;
  /** What the user loses below this version. */
  effect: Bilingual;
};

/** Newest first: the first missing entry is the version the user has to reach. */
export const WEBVIEW2_THRESHOLDS: readonly WebView2Threshold[] = [
  {
    version: '123.0.0.0',
    feature: 'field-sizing',
    effect: ['输入框随内容自动增高', 'the composer growing with its text'],
  },
  {
    version: '111.0.0.0',
    feature: 'color-mix()',
    effect: ['聚焦时的光晕', 'the focus glow'],
  },
  {
    version: '105.0.0.0',
    feature: ':has() / @container',
    effect: ['选中态与窄窗自适应样式', 'selected-state and narrow-window styles'],
  },
  {
    version: '94.0.0.0',
    feature: 'es2022',
    effect: ['界面脚本运行', 'the interface scripts running'],
  },
];

/**
 * Install-time policy for the runtime, from `docs/user-guide.md:11`. The interface
 * states it because it is the one offline limit of an otherwise offline product:
 * installing on a machine without the runtime needs a network once.
 */
export const WEBVIEW2_INSTALL_NOTICE: Bilingual = [
  '安装包不含 WebView2 运行时：本机缺少时，安装器需要联网下载；离线机器请先自行安装 Microsoft Edge WebView2 Runtime。',
  'The installer does not carry the WebView2 runtime: it downloads it when the machine has none, so an offline machine needs the Microsoft Edge WebView2 Runtime installed first.',
];

/** Dotted numeric parts, or null when the runtime reported something else. */
export function parseWebViewVersion(value: string | null | undefined): number[] | null {
  if (typeof value !== 'string') return null;
  const parts = value.trim().split('.');
  if (!parts.length || parts.some((part) => !/^\d+$/.test(part))) return null;
  return parts.map(Number);
}

/** Missing parts count as zero, because the runtime does not always report four. */
function compareVersionParts(left: readonly number[], right: readonly number[]): -1 | 0 | 1 {
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const difference = (left[index] ?? 0) - (right[index] ?? 0);
    if (difference !== 0) return difference < 0 ? -1 : 1;
  }
  return 0;
}

/**
 * Numeric part-by-part comparison, so `9.0.0.0` reads as older than `105.0.0.0`.
 * Returns null when either side is not a version, so a caller cannot read a
 * comparison that was never computed.
 */
export function compareWebViewVersions(left: string, right: string): -1 | 0 | 1 | null {
  const a = parseWebViewVersion(left);
  const b = parseWebViewVersion(right);
  if (!a || !b) return null;
  return compareVersionParts(a, b);
}

/**
 * The runtime version a WebView2 user agent reports. WebView2 sends the Edge user
 * agent, whose `Edg/<version>` token is the runtime (Chromium) version the shell
 * is actually drawing in: reading it here lets the shell warn on the way in
 * instead of waiting for the reader to open the diagnostics page.
 */
export function webView2VersionFromUserAgent(userAgent: string): string | null {
  const match = /Edg\/(\d+(?:\.\d+)*)/.exec(typeof userAgent === 'string' ? userAgent : '');
  return match ? match[1] : null;
}
export type WebView2Compatibility = {
  /** `unknown` means the runtime reported no version at all, which is not a pass. */
  state: 'unknown' | 'outdated' | 'supported';
  /** The reported version unchanged, or null when there was nothing to read. */
  version: string | null;
  /** Thresholds this runtime is too old for, newest first. */
  missing: WebView2Threshold[];
};

export function webView2Compatibility(
  version: string | null | undefined,
  thresholds: readonly WebView2Threshold[] = WEBVIEW2_THRESHOLDS,
): WebView2Compatibility {
  const reported = typeof version === 'string' ? version.trim() : '';
  const parsed = parseWebViewVersion(reported);
  if (!parsed) return { state: 'unknown', version: null, missing: [] };
  const missing = thresholds.filter((threshold) => {
    const required = parseWebViewVersion(threshold.version);
    return required ? compareVersionParts(parsed, required) < 0 : false;
  });
  return { state: missing.length ? 'outdated' : 'supported', version: reported, missing };
}

export type WebView2Notice = { level: 'info' | 'warn'; text: string };

/**
 * One line for the diagnostics page. An older runtime names the newest feature it
 * cannot render and the exact version that would restore it, instead of saying
 * "unsupported" and leaving the user to guess what to update.
 */
export function webView2Notice(
  compatibility: WebView2Compatibility,
  language: Language = 'zh-CN',
): WebView2Notice {
  const pick = (pair: Bilingual) => (language === 'en' ? pair[1] : pair[0]);
  if (compatibility.state === 'unknown')
    return {
      level: 'info',
      text: pick([
        '本机未回报 WebView2 版本，无法判断界面特性是否完整。',
        'This machine reported no WebView2 version, so the interface cannot tell whether every feature it uses is available.',
      ]),
    };
  const version = compatibility.version ?? '';
  if (compatibility.state === 'supported')
    return {
      level: 'info',
      text: pick([
        `本机 WebView2 ${version}，界面所需的样式与脚本特性都可用。`,
        `WebView2 ${version} is installed and supports every style and script feature the interface uses.`,
      ]),
    };
  const newest = compatibility.missing[0];
  return {
    level: 'warn',
    text:
      language === 'en'
        ? `WebView2 ${version} is older than ${newest.version}, the newest feature the interface uses (${newest.feature}), so ${compatibility.missing.map((threshold) => pick(threshold.effect)).join(', ')} will not apply. Update the Microsoft Edge WebView2 Runtime and restart the app. For an offline machine, prepare the matching runtime installer first.`
        : `本机 WebView2 ${version} 低于界面所用的最新特性 ${newest.version}（${newest.feature}）：${compatibility.missing.map((threshold) => pick(threshold.effect)).join('、')}不会生效。请更新 Microsoft Edge WebView2 Runtime 后重启应用；离线机器请先准备对应的 Runtime 安装包。`,
  };
}

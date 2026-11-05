/**
 * The interface language, outside React.
 *
 * Components read it from context, but helpers that are not components (kernel
 * error text, formatters called outside render) still have to answer in the
 * user's language. This module is the one place that remembers it, and it has no
 * React dependency so both the browser bundle and the plain-node tests can use
 * it.
 */
export type Language = 'zh-CN' | 'en';

let active: Language = 'zh-CN';

export function setActiveLanguage(language: Language) {
  active = language;
}

export function activeLanguage(): Language {
  return active;
}

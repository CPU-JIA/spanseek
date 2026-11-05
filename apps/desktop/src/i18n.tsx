import { createContext, useCallback, useContext } from 'react';
import { type Language } from './language';

export type { Language } from './language';
export { activeLanguage, setActiveLanguage } from './language';

export const LanguageContext = createContext<Language>('zh-CN');

export function useI18n() {
  const language = useContext(LanguageContext);
  const t = useCallback((zh: string, en: string) => (language === 'en' ? en : zh), [language]);
  return { language, t };
}

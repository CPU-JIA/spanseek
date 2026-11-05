const options: Intl.DateTimeFormatOptions = {
  month: 'short',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
};
const dates = {
  'zh-CN': new Intl.DateTimeFormat('zh-CN', options),
  en: new Intl.DateTimeFormat('en', options),
};
const detailedOptions: Intl.DateTimeFormatOptions = {
  year: 'numeric',
  month: 'short',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
};
const detailedDates = {
  'zh-CN': new Intl.DateTimeFormat('zh-CN', detailedOptions),
  en: new Intl.DateTimeFormat('en', detailedOptions),
};
export function displayDate(value: string, language: 'zh-CN' | 'en' = 'zh-CN') {
  return dates[language].format(new Date(value));
}

/** Document properties keep the year; list rows use the shorter form. */
export function displayDateTime(value: string, language: 'zh-CN' | 'en' = 'zh-CN') {
  return detailedDates[language].format(new Date(value));
}

/** Windows extended paths stay canonical in storage; simplify only their display. */
export function displayPath(value: string) {
  if (/^\\\\\?\\UNC\\/i.test(value)) return `\\\\${value.slice(8)}`;
  if (/^\\\\\?\\[a-z]:\\/i.test(value)) return value.slice(4);
  return value;
}

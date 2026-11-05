import { useState } from 'react';
import { message } from '../api';

/** The caller is keyed by workspace/task, so a draft never crosses workspace identities. */
export function useTaskDraft<T>(key: string, initial: () => T, parse: (raw: string) => T) {
  const [loaded] = useState(() => {
    try {
      const raw = localStorage.getItem(key);
      return { value: raw === null ? initial() : parse(raw), error: '' };
    } catch (error) {
      return { value: initial(), error: message(error) };
    }
  });
  const [value, setValue] = useState(loaded.value);
  const [error, setError] = useState(loaded.error);
  const [unreadable, setUnreadable] = useState(!!loaded.error);
  function save(next: T, replaceUnreadable = false) {
    setValue(next);
    if (unreadable && !replaceUnreadable) return false;
    try {
      localStorage.setItem(key, JSON.stringify(next));
      setError('');
      setUnreadable(false);
      return true;
    } catch (failure) {
      setError(message(failure));
      return false;
    }
  }
  return { value, save, error, unreadable };
}

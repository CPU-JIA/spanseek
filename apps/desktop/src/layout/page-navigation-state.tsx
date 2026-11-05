import { createContext, useCallback, useContext, useRef, type SetStateAction } from 'react';

export type PageNavigationState = Readonly<Record<string, unknown>>;
export const PageNavigationContext = createContext<{
  state: PageNavigationState;
  update: (key: string, change: (previous: unknown) => unknown) => void;
} | null>(null);

/** Small page selections/filters survive navigation; drafts keep their existing owners. */
export function usePageNavigationState<T>(key: string, initial: T | (() => T)) {
  const context = useContext(PageNavigationContext);
  if (!context) throw new Error('Page navigation context is missing');
  const fallback = useRef<{ value: T } | null>(null);
  if (!fallback.current)
    fallback.current = { value: typeof initial === 'function' ? (initial as () => T)() : initial };
  const initialValue = fallback.current.value;
  const value = key in context.state ? (context.state[key] as T) : initialValue;
  const update = context.update;
  const setValue = useCallback(
    (change: SetStateAction<T>) => {
      update(key, (old) => {
        const previous = old === undefined ? initialValue : (old as T);
        return typeof change === 'function' ? (change as (value: T) => T)(previous) : change;
      });
    },
    [key, update, initialValue],
  );
  return [value, setValue] as const;
}

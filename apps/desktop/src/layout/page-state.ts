export type ListDisplayState = 'loading' | 'error' | 'empty' | 'content' | 'content-with-error';

/**
 * A list page is only empty after the first page has arrived. Keeping this
 * decision in one place prevents a failed or still-loading read from being
 * presented as "0 items", and lets an error coexist with rows already on screen.
 */
export function resolveListDisplayState({
  hasPage,
  itemCount,
  loading,
  error,
}: {
  hasPage: boolean;
  itemCount: number;
  loading: boolean;
  error: string | null;
}): ListDisplayState {
  if (itemCount > 0) return error ? 'content-with-error' : 'content';
  if (error) return 'error';
  if (loading || !hasPage) return 'loading';
  return 'empty';
}

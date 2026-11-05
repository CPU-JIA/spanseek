import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { usePagedList, type PagedListController } from './Pagination.tsx';
import type { CursorPage } from './pagination.ts';
import { usePageNavigationState } from './page-navigation-state';

/** Rebuild a cursor path on return; never let the temporary first page overwrite it. */
export function usePageNavigationPaging<T>(
  scope: string | null,
  key: string,
  read: (cursor: string | null, size: number) => Promise<CursorPage<T>>,
  active = true,
  visible = active,
) {
  const [savedPage, setSavedPage] = usePageNavigationState(`${key}.page`, 1);
  const [savedSize, setSavedSize] = usePageNavigationState(`${key}.size`, 10);
  const [scroll, setScroll] = usePageNavigationState(`${key}.scroll`, 0);
  const element = useRef<HTMLDivElement>(null);
  const currentScope = active ? scope : null;
  const scopeRef = useRef(currentScope);
  scopeRef.current = currentScope;
  const restoringScope = useRef<string | null>(null);
  const epoch = useRef(0);
  const readSequence = useRef(0);
  const lastReadFailed = useRef(false);
  const [restoredScope, setRestoredScope] = useState<string | null>(null);
  const list = usePagedList<T>(
    currentScope,
    async (cursor, size) => {
      const sequence = ++readSequence.current;
      try {
        const page = await read(cursor, size);
        if (sequence === readSequence.current) lastReadFailed.current = false;
        return page;
      } catch (error) {
        if (sequence === readSequence.current) lastReadFailed.current = true;
        throw error;
      }
    },
    savedSize,
  );
  const ready = active && scope !== null && restoredScope === scope && !list.loading;

  useEffect(() => {
    ++epoch.current;
    restoringScope.current = null;
    setRestoredScope(null);
  }, [currentScope]);

  useEffect(() => {
    if (
      !active ||
      scope === null ||
      !list.page ||
      list.loading ||
      list.error ||
      restoredScope === scope ||
      restoringScope.current === scope
    )
      return;
    restoringScope.current = scope;
    const requestEpoch = epoch.current;
    void (savedPage > 1 ? list.goTo(savedPage) : Promise.resolve(true))
      .then((reached) => {
        if (scopeRef.current === scope && epoch.current === requestEpoch) {
          // A failed read must leave the saved destination intact for retry.
          // A clean end of a shortened list can settle on its last real page.
          if (reached || !lastReadFailed.current) setRestoredScope(scope);
        }
      })
      .finally(() => {
        if (scopeRef.current === scope && epoch.current === requestEpoch)
          restoringScope.current = null;
      });
  }, [active, scope, list.page, list.loading, list.error, restoredScope]);

  useEffect(() => {
    if (ready && list.page && !list.error) setSavedPage(list.index);
  }, [ready, list.page, list.index, list.error, setSavedPage]);

  useLayoutEffect(() => {
    if (ready && visible && element.current) element.current.scrollTop = scroll;
  }, [ready, visible, list.page]);

  const pager: PagedListController<T> = {
    ...list,
    loading: list.loading || (active && !!list.page && !list.error && !ready),
    loadFirst: () => {
      setSavedPage(1);
      setScroll(0);
      return list.loadFirst();
    },
    setSize: (size) => {
      setSavedPage(1);
      setSavedSize(size);
      setScroll(0);
      return list.setSize(size);
    },
    next: () => {
      setScroll(0);
      return list.next();
    },
    previous: () => {
      setScroll(0);
      return list.previous();
    },
    goTo: (page) => {
      setScroll(0);
      return list.goTo(page);
    },
  };
  return {
    pager,
    ready,
    element,
    rememberScroll: () => {
      if (ready && visible && element.current) setScroll(element.current.scrollTop);
    },
  };
}

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useI18n, type Language } from '../i18n';
import { Select } from '../ui/Select';
import {
  DEFAULT_PAGE_SIZE,
  PAGE_SIZE_OPTIONS,
  PagedList,
  clampPageSize,
  pageIndexText,
  pageStatusText,
  pageWindow,
  shouldRenderPagination,
  totalPageText,
  type CursorPage,
  type PagedSnapshot,
  type PageSize,
  type PaginationVariant,
} from './pagination.ts';
import './pagination.css';

export type PagedListController<T> = PagedSnapshot<T> & {
  loadFirst: () => Promise<boolean>;
  refresh: () => Promise<boolean>;
  next: () => Promise<boolean>;
  previous: () => Promise<boolean>;
  goTo: (index: number) => Promise<boolean>;
  setSize: (size: number) => Promise<boolean>;
  updateItems: (mutator: (items: readonly T[]) => T[]) => void;
};

/**
 * One paged list for one scope. The scope key carries the workspace identity and
 * every filter/sort input, so a different scope is a different list with its own
 * cursor stack; a null scope means "no workspace yet" and reads nothing.
 */
export function usePagedList<T>(
  scope: string | null,
  read: (cursor: string | null, size: number) => Promise<CursorPage<T>>,
  initialSize: number = DEFAULT_PAGE_SIZE,
  numbered = false,
): PagedListController<T> {
  const reader = useRef(read);
  reader.current = read;
  const list = useMemo(
    () =>
      new PagedList<T>(
        (cursor, size) => reader.current(cursor, size),
        scope ?? '',
        clampPageSize(initialSize),
        numbered,
      ),
    [scope, numbered],
  );
  const state = useSyncExternalStore(list.subscribe, list.getSnapshot, () => list.getSnapshot());
  useEffect(() => {
    if (scope === null) return;
    void list.loadFirst();
    return () => list.invalidate('scope');
  }, [list, scope]);
  return {
    ...state,
    loadFirst: () => list.loadFirst(),
    refresh: () => list.refresh(),
    next: () => list.next(),
    previous: () => list.previous(),
    goTo: (index: number) => list.goTo(index),
    setSize: (size: number) => list.setSize(size),
    updateItems: (mutator) => list.updateItems(mutator),
  };
}

export type PaginationProps = {
  index: number;
  size: number;
  /** Rows on the page right now; the page number is real, the total usually is not. */
  count: number;
  /** Only when the list reports a true total; otherwise no page count is shown. */
  total?: number | null;
  loading?: boolean;
  canPrevious: boolean;
  canNext: boolean;
  disabled?: boolean;
  className?: string;
  /**
   * `footer` (default) is the page's last row: it sticks to the bottom of the
   * pane and draws the single top rule. `inline` continues a list, picker or
   * filter popover, so it draws no rule.
   */
  variant?: PaginationVariant;
  onSize: (size: PageSize) => void;
  onPrevious: () => void;
  onNext: () => void;
  onGoTo?: (page: number) => void;
};

export function Pagination({
  index,
  size,
  count,
  total,
  loading,
  canPrevious,
  canNext,
  disabled,
  className,
  variant,
  onSize,
  onPrevious,
  onNext,
  onGoTo,
}: PaginationProps) {
  const { t, language } = useI18n();
  const shape = variant ?? 'footer';
  const [jump, setJump] = useState(String(index));
  useEffect(() => setJump(String(index)), [index]);
  // The footer is part of the page: an empty, non-navigable list still renders
  // it ("共 0 项" with both step buttons disabled) instead of hiding the pager.
  if (!shouldRenderPagination({ count, canPrevious, canNext, variant: shape })) return null;
  const window = pageWindow(index, size, count);
  const status =
    count === 0 && total != null && total > 0
      ? t('本页暂无条目', 'No items on this page')
      : pageStatusText(window, language as Language);
  const blocked = disabled || loading;
  return (
    <nav
      className={['pagination', `pagination--${shape}`, className].filter(Boolean).join(' ')}
      aria-label={t('分页', 'Pagination')}
    >
      <span className="pagination-window" role="status">
        {status}
        {total == null || total === 0 ? '' : t(` · 共 ${total} 项`, ` · ${total} total`)}
      </span>
      <div className="choice-field pagination-size">
        <span className="pagination-size-label">{t('每页', 'Per page')}</span>
        <Select
          value={size}
          disabled={blocked}
          aria-label={t('每页条数', 'Items per page')}
          onValueChange={(value) => onSize(clampPageSize(Number(value)))}
        >
          {PAGE_SIZE_OPTIONS.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </Select>
        <span className="pagination-size-unit">{t('项', 'items')}</span>
      </div>
      <span className="pagination-index">
        {pageIndexText(index, language)}
        <span className="pagination-total">
          {total === null || total === undefined
            ? ''
            : ` · ${totalPageText(total, size, language as Language)}`}
        </span>
      </span>
      <button
        type="button"
        className="secondary-button pagination-step"
        aria-label={t('上一页', 'Previous page')}
        disabled={blocked || !canPrevious}
        onClick={onPrevious}
      >
        <ChevronLeft size={15} aria-hidden="true" />
        <span className="pagination-step-label">{t('上一页', 'Previous')}</span>
      </button>
      {onGoTo && total != null && total > size && shape === 'footer' && (
        <form
          className="pagination-jump"
          onSubmit={(event) => {
            event.preventDefault();
            const page = Number(jump);
            if (!blocked && Number.isInteger(page) && page >= 1 && page <= Math.ceil(total / size))
              onGoTo(page);
          }}
        >
          <input
            className="ui-input"
            type="number"
            min={1}
            max={Math.ceil(total / size)}
            value={jump}
            disabled={blocked}
            aria-label={t('跳转页码', 'Page to jump to')}
            onChange={(event) => setJump(event.target.value)}
          />
          <button
            type="submit"
            className="secondary-button"
            disabled={blocked}
            aria-label={t('跳转到指定页', 'Go to page')}
          >
            {t('跳转', 'Go')}
          </button>
        </form>
      )}
      <button
        type="button"
        className="secondary-button pagination-step"
        aria-label={t('下一页', 'Next page')}
        disabled={blocked || !canNext}
        onClick={onNext}
      >
        <span className="pagination-step-label">{t('下一页', 'Next')}</span>
        <ChevronRight size={15} aria-hidden="true" />
      </button>
    </nav>
  );
}

/** Pager for a `usePagedList` result; keeps every page wired the same way. */
export function ListPagination<T>({
  list,
  total,
  disabled,
  className,
  variant,
}: {
  list: PagedListController<T>;
  total?: number | null;
  disabled?: boolean;
  className?: string;
  variant?: PaginationVariant;
}) {
  return (
    <Pagination
      index={list.index}
      size={list.size}
      count={list.page?.items.length ?? 0}
      total={total ?? list.page?.total}
      loading={list.loading}
      canPrevious={list.canPrevious}
      canNext={list.canNext}
      disabled={disabled}
      className={className}
      variant={variant}
      onSize={(size) => void list.setSize(size)}
      onPrevious={() => void list.previous()}
      onNext={() => void list.next()}
      onGoTo={(page) => void list.goTo(page)}
    />
  );
}

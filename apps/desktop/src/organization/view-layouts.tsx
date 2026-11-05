import { useMemo, useState } from 'react';
import { CalendarDays, ExternalLink, LayoutList, Tag as TagIcon, Trash2, X } from 'lucide-react';
import type {
  InboxState,
  ObjectMetadata,
  Tag,
  ViewConfig,
  ViewLayout,
  ViewNote,
  ViewNoteProperty,
  ViewProperty,
} from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { Select } from '../ui/Select';
import { TagPills } from '../ui/TagPills';
import { tagToneClass } from '../ui/tag-color';
import './view-layouts.css';

type Translate = (zh: string, en: string) => string;
type ViewMetadataItem = Pick<ViewNote, 'note' | 'metadata'> &
  Partial<Pick<ViewNote, 'properties' | 'properties_truncated' | 'properties_error'>>;

export function layoutLabel(layout: ViewLayout, t: Translate) {
  switch (layout) {
    case 'table':
      return t('表格视图', 'Table view');
    case 'board':
      return t('看板视图', 'Board view');
    case 'calendar':
      return t('日历视图', 'Calendar view');
    case 'timeline':
      return t('时间线视图', 'Timeline view');
    default:
      return t('列表视图', 'List view');
  }
}

export function viewStatusLabel(value: InboxState | null, t: Translate) {
  return value === 'pending'
    ? t('待整理', 'Inbox')
    : value === 'snoozed'
      ? t('稍后整理', 'Later')
      : t('已整理', 'Organized');
}

export function propertyLabel(property: ViewProperty, t: Translate) {
  switch (property) {
    case 'inbox_state':
      return t('整理状态', 'State');
    case 'tags':
      return t('标签', 'Tags');
    case 'archived':
      return t('归档', 'Archived');
    case 'search_enabled':
      return t('搜索收录', 'Search');
    case 'updated_at':
      return t('最后编辑', 'Last edited');
  }
}

export function collectCustomPropertyKeys(items: readonly ViewNote[]) {
  const keys = new Set<string>();
  for (const item of items) for (const property of item.properties) keys.add(property.key);
  return [...keys].sort((left, right) => left.localeCompare(right));
}

function customProperty(item: ViewNote, key: string) {
  return item.properties.find((property) => property.key === key);
}

function customPropertyText(
  property: ViewNoteProperty | undefined,
  language: 'zh-CN' | 'en',
  t: Translate,
) {
  if (!property) return t('未设置', 'Not set');
  if (property.kind === 'null') return t('空值', 'Empty');
  if (property.kind === 'nested') return t('嵌套结构', 'Nested value');
  if (property.kind === 'boolean')
    return property.value === 'true' ? t('是', 'True') : t('否', 'False');
  if (property.kind === 'date') {
    const date = new Date(`${property.value}T12:00:00`);
    if (!Number.isNaN(date.getTime())) {
      return new Intl.DateTimeFormat(language === 'en' ? 'en-US' : 'zh-CN', {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
      }).format(date);
    }
  }
  if (property.kind === 'datetime') {
    const date = new Date(property.value);
    if (!Number.isNaN(date.getTime())) return displayDate(date.toISOString(), language);
  }
  return property.truncated ? `${property.value}…` : property.value;
}

function propertyKindLabel(kind: ViewNoteProperty['kind'], t: Translate) {
  switch (kind) {
    case 'text':
      return t('文本', 'Text');
    case 'number':
      return t('数字', 'Number');
    case 'boolean':
      return t('是/否', 'Boolean');
    case 'date':
      return t('日期', 'Date');
    case 'datetime':
      return t('日期时间', 'Date & time');
    case 'null':
      return t('空值', 'Empty');
    case 'list':
      return t('列表', 'List');
    case 'nested':
      return t('嵌套', 'Nested');
  }
}

function itemDayKey(item: ViewNote, dateProperty: string | null) {
  if (!dateProperty) {
    const key = dayKey(item.note.updated_at);
    return key === 'unknown' ? null : key;
  }
  const property = customProperty(item, dateProperty);
  if (!property) return null;
  if (property.kind === 'date') {
    return /^\d{4}-\d{2}-\d{2}$/.test(property.value) ? property.value : null;
  }
  if (property.kind !== 'datetime') return null;
  const key = dayKey(property.value);
  return key === 'unknown' ? null : key;
}

function CustomPropertySummary({
  item,
  keys,
  language,
  t,
  limit = 3,
}: {
  item: ViewNote;
  keys: readonly string[];
  language: 'zh-CN' | 'en';
  t: Translate;
  limit?: number;
}) {
  const selected = keys
    .map((key) => customProperty(item, key))
    .filter((property): property is ViewNoteProperty => Boolean(property))
    .slice(0, limit);
  if (!selected.length) return null;
  return (
    <span className="view-custom-property-summary">
      {selected.map((property) => (
        <span className="view-custom-property" key={property.key}>
          <strong>{property.key}</strong>
          <span className="view-custom-property-value">
            {customPropertyText(property, language, t)}
          </span>
        </span>
      ))}
    </span>
  );
}

function customPropertyCell(key: string, item: ViewNote, language: 'zh-CN' | 'en', t: Translate) {
  const property = customProperty(item, key);
  if (item.properties_error) {
    return (
      <span
        className="view-custom-cell error"
        title={t('Frontmatter 无法解析', 'Frontmatter could not be parsed')}
      >
        {t('读取失败', 'Unreadable')}
      </span>
    );
  }
  const value = customPropertyText(property, language, t);
  return (
    <span
      className={`view-custom-cell${property ? '' : ' missing'}`}
      title={property?.truncated ? t('属性值已截断', 'Property value truncated') : value}
    >
      {value}
    </span>
  );
}

function formatDate(value: string, language: 'zh-CN' | 'en') {
  return displayDate(value, language);
}

function dayKey(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return 'unknown';
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function monthKey(date: Date) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`;
}

function monthLabel(month: string, language: 'zh-CN' | 'en') {
  const [year, raw] = month.split('-').map(Number);
  const date = new Date(year, (raw || 1) - 1, 1);
  return new Intl.DateTimeFormat(language === 'en' ? 'en-US' : 'zh-CN', {
    year: 'numeric',
    month: 'long',
  }).format(date);
}

function monthGrid(month: string) {
  const [year, rawMonth] = month.split('-').map(Number);
  const first = new Date(year, (rawMonth || 1) - 1, 1);
  const start = new Date(first);
  const weekday = first.getDay();
  // Monday-first grid, matching a typical planning calendar.
  const offset = (weekday + 6) % 7;
  start.setDate(first.getDate() - offset);
  return Array.from({ length: 42 }, (_, index) => {
    const date = new Date(start);
    date.setDate(start.getDate() + index);
    return date;
  });
}

function propertyValue(
  property: ViewProperty,
  item: ViewNote,
  language: 'zh-CN' | 'en',
  t: Translate,
) {
  switch (property) {
    case 'inbox_state':
      return viewStatusLabel(item.note.inbox_state, t);
    case 'tags':
      return item.metadata.tags.length
        ? item.metadata.tags.map((tag) => `#${tag.display_name}`).join(' · ')
        : t('无标签', 'No tags');
    case 'archived':
      return item.metadata.archived ? t('已归档', 'Archived') : t('未归档', 'Active');
    case 'search_enabled':
      return item.metadata.search_enabled ? t('已收录', 'Included') : t('不收录', 'Excluded');
    case 'updated_at':
      return formatDate(item.note.updated_at, language);
  }
}

function PropertyCell({
  property,
  item,
  language,
  t,
}: {
  property: ViewProperty;
  item: ViewNote;
  language: 'zh-CN' | 'en';
  t: Translate;
}) {
  if (property === 'tags') {
    return item.metadata.tags.length ? (
      <TagPills tags={item.metadata.tags} />
    ) : (
      <>{t('无标签', 'No tags')}</>
    );
  }
  return <>{propertyValue(property, item, language, t)}</>;
}
function propertyColumnWidth(property: ViewProperty) {
  switch (property) {
    case 'tags':
      return '160px';
    case 'updated_at':
      return '140px';
    case 'inbox_state':
      return '110px';
    case 'archived':
    case 'search_enabled':
      return '100px';
  }
}

export function ViewLayoutView({
  view,
  items,
  selectedId,
  onSelect,
  onOpenNote,
  onChanged,
}: {
  view: { layout: string; config: ViewConfig };
  items: ViewNote[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onOpenNote: (id: string) => void;
  onChanged: () => void;
}) {
  if (view.layout === 'table') {
    return (
      <TableView
        view={view}
        items={items}
        selectedId={selectedId}
        onSelect={onSelect}
        onOpenNote={onOpenNote}
      />
    );
  }
  if (view.layout === 'board') {
    return (
      <BoardView
        view={view}
        items={items}
        selectedId={selectedId}
        onSelect={onSelect}
        onOpenNote={onOpenNote}
      />
    );
  }
  if (view.layout === 'calendar') {
    return (
      <CalendarView
        view={view}
        items={items}
        selectedId={selectedId}
        onSelect={onSelect}
        onOpenNote={onOpenNote}
      />
    );
  }
  if (view.layout === 'timeline') {
    return (
      <TimelineView
        view={view}
        items={items}
        selectedId={selectedId}
        onSelect={onSelect}
        onOpenNote={onOpenNote}
      />
    );
  }
  return (
    <ListView
      view={view}
      items={items}
      selectedId={selectedId}
      onSelect={onSelect}
      onOpenNote={onOpenNote}
    />
  );
}

function NoteTitle({ item, onOpenNote }: { item: ViewNote; onOpenNote: (id: string) => void }) {
  const { t } = useI18n();
  return (
    <button className="view-note-title" onClick={() => onOpenNote(item.note.id)}>
      <strong>{item.note.title || t('未命名笔记', 'Untitled note')}</strong>
      {item.properties_error && (
        <span className="view-note-warning">{t('YAML 错误', 'YAML error')}</span>
      )}
      <ExternalLink size={13} aria-hidden="true" />
    </button>
  );
}

function NotePreview({ item }: { item: ViewNote }) {
  const { t } = useI18n();
  return (
    <p className="view-note-preview">{item.note.preview || t('还没有正文。', 'No content yet.')}</p>
  );
}

function ListView({
  view,
  items,
  selectedId,
  onSelect,
  onOpenNote,
}: {
  view: { config: ViewConfig };
  items: ViewNote[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onOpenNote: (id: string) => void;
}) {
  const { t, language } = useI18n();
  return (
    <div className="view-list">
      {items.map((item) => (
        <div
          className={`view-list-row${selectedId === item.note.id ? ' selected' : ''}`}
          key={item.note.id}
        >
          <button
            className="view-row-select"
            onClick={() => onSelect(item.note.id)}
            aria-label={t('查看属性', 'Show properties')}
          />
          <div>
            <NoteTitle item={item} onOpenNote={onOpenNote} />
            <NotePreview item={item} />
            <small className="view-note-meta">
              <span>
                {viewStatusLabel(item.note.inbox_state, t)} ·{' '}
                {formatDate(item.note.updated_at, language)}
              </span>
              <TagPills
                tags={item.metadata.tags}
                limit={4}
                moreLabel={(count) => t(`另有 ${count} 个标签`, `${count} more tags`)}
              />
              <CustomPropertySummary
                item={item}
                keys={view.config.custom_properties}
                language={language}
                t={t}
                limit={4}
              />
            </small>
          </div>
        </div>
      ))}
    </div>
  );
}

function TableView({
  view,
  items,
  selectedId,
  onSelect,
  onOpenNote,
}: {
  view: { config: ViewConfig };
  items: ViewNote[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onOpenNote: (id: string) => void;
}) {
  const { t, language } = useI18n();
  const columns = view.config.visible_properties;
  const customColumns = view.config.custom_properties;
  return (
    <div className="view-table-wrap">
      <table className="view-table">
        <thead>
          <tr>
            <th>{t('笔记', 'Note')}</th>
            {columns.map((property) => (
              <th key={property}>{propertyLabel(property, t)}</th>
            ))}
            {customColumns.map((key) => (
              <th key={`custom:${key}`}>{key}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {items.map((item) => (
            <tr
              key={item.note.id}
              className={selectedId === item.note.id ? 'selected' : undefined}
              tabIndex={0}
              aria-selected={selectedId === item.note.id}
              onClick={() => onSelect(item.note.id)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  onSelect(item.note.id);
                }
              }}
            >
              <td>
                <NoteTitle item={item} onOpenNote={onOpenNote} />
              </td>
              {columns.map((property) => (
                <td
                  key={property}
                  data-label={propertyLabel(property, t)}
                  style={{ minWidth: propertyColumnWidth(property) }}
                >
                  <PropertyCell property={property} item={item} language={language} t={t} />
                </td>
              ))}
              {customColumns.map((key) => (
                <td
                  key={`custom:${key}`}
                  data-label={key}
                  style={{ minWidth: '120px', maxWidth: '220px' }}
                >
                  {customPropertyCell(key, item, language, t)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function BoardView({
  view,
  items,
  selectedId,
  onSelect,
  onOpenNote,
}: {
  view: { config: ViewConfig };
  items: ViewNote[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onOpenNote: (id: string) => void;
}) {
  const { t, language } = useI18n();
  const groups = useMemo(() => {
    if (view.config.board_group_by === 'tag') {
      const tags = new Map<string, Tag>();
      for (const item of items) for (const tag of item.metadata.tags) tags.set(tag.id, tag);
      const sorted = [...tags.values()].sort((a, b) =>
        a.display_name.localeCompare(b.display_name),
      );
      return [
        ...sorted.map((tag) => ({
          id: tag.id,
          label: tag.display_name,
          tone: tagToneClass(tag.id),
          items: items.filter((item) =>
            item.metadata.tags.some((candidate) => candidate.id === tag.id),
          ),
        })),
        {
          id: '__untagged',
          label: t('无标签', 'No tags'),
          items: items.filter((item) => item.metadata.tags.length === 0),
        },
      ];
    }
    return [
      {
        id: 'pending',
        label: t('待整理', 'Inbox'),
        items: items.filter((item) => item.note.inbox_state === 'pending'),
      },
      {
        id: 'snoozed',
        label: t('稍后整理', 'Later'),
        items: items.filter((item) => item.note.inbox_state === 'snoozed'),
      },
      {
        id: 'processed',
        label: t('已整理', 'Organized'),
        items: items.filter((item) => item.note.inbox_state === 'processed'),
      },
    ];
  }, [items, t, view.config.board_group_by]);
  return (
    <div className="view-board">
      {groups.map((group) => (
        <section
          className={`view-board-column ${'tone' in group ? group.tone : ''}`}
          key={group.id}
          aria-label={group.label}
        >
          <header>
            <h4>{group.label}</h4>
            <span>{group.items.length}</span>
          </header>
          <div className="view-board-stack">
            {group.items.map((item) => (
              <article
                className={`view-board-card${selectedId === item.note.id ? ' selected' : ''}`}
                key={item.note.id}
              >
                <button
                  className="view-card-select"
                  aria-label={t('查看属性：', 'Show properties: ') + item.note.title}
                  aria-pressed={selectedId === item.note.id}
                  onClick={() => onSelect(item.note.id)}
                />
                <div className="view-card-content">
                  <NoteTitle item={item} onOpenNote={onOpenNote} />
                  <NotePreview item={item} />
                  {(item.metadata.tags.length > 0 || view.config.custom_properties.length > 0) && (
                    <div className="view-card-meta">
                      {item.metadata.tags.length > 0 && (
                        <TagPills
                          tags={item.metadata.tags}
                          limit={3}
                          moreLabel={(count) => t(`另有 ${count} 个标签`, `${count} more tags`)}
                        />
                      )}
                      <CustomPropertySummary
                        item={item}
                        keys={view.config.custom_properties}
                        language={language}
                        t={t}
                        limit={3}
                      />
                    </div>
                  )}
                </div>
              </article>
            ))}
            {!group.items.length && (
              <p className="view-empty-column">{t('没有笔记', 'No notes')}</p>
            )}
          </div>
        </section>
      ))}
    </div>
  );
}

function CalendarView({
  view,
  items,
  selectedId,
  onSelect,
  onOpenNote,
}: {
  view: { config: ViewConfig };
  items: ViewNote[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onOpenNote: (id: string) => void;
}) {
  const { t, language } = useI18n();
  const [openDay, setOpenDay] = useState<string | null>(null);
  const currentMonth = monthKey(new Date());
  const month = view.config.calendar_month ?? currentMonth;
  const days = monthGrid(month);
  const dateProperty = view.config.date_property;
  const { groups, unscheduled } = useMemo(() => {
    const map = new Map<string, ViewNote[]>();
    const unplaced: ViewNote[] = [];
    for (const item of items) {
      const key = itemDayKey(item, dateProperty);
      if (key) map.set(key, [...(map.get(key) ?? []), item]);
      else unplaced.push(item);
    }
    return { groups: map, unscheduled: unplaced };
  }, [dateProperty, items]);
  const today = dayKey(new Date().toISOString());
  const openDayItems = openDay ? (groups.get(openDay) ?? []) : [];
  const openDayLabel = openDay
    ? new Intl.DateTimeFormat(language === 'en' ? 'en-US' : 'zh-CN', {
        year: 'numeric',
        month: 'long',
        day: 'numeric',
        weekday: 'long',
      }).format(new Date(`${openDay}T12:00:00`))
    : '';
  const weekday =
    language === 'en'
      ? ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']
      : ['一', '二', '三', '四', '五', '六', '日'];
  return (
    <div className="view-calendar">
      <div className="view-calendar-title">
        <CalendarDays size={16} />
        <strong>{monthLabel(month, language)}</strong>
        <span>
          {dateProperty
            ? t(`按属性 ${dateProperty}`, `By ${dateProperty}`)
            : t('按最后编辑日期', 'By last edited date')}
        </span>
      </div>
      {openDay && (
        <section
          className="view-calendar-agenda"
          id="view-calendar-agenda"
          aria-label={t('当日笔记', 'Notes on this day')}
        >
          <header>
            <div>
              <strong>{openDayLabel}</strong>
              <small>{t(`${openDayItems.length} 篇笔记`, `${openDayItems.length} notes`)}</small>
            </div>
            <button
              className="icon-button"
              aria-label={t('关闭当日清单', 'Close day list')}
              onClick={() => setOpenDay(null)}
            >
              <X size={14} />
            </button>
          </header>
          <div className="view-calendar-agenda-list">
            {openDayItems.map((item) => (
              <div
                className={`view-calendar-agenda-item${selectedId === item.note.id ? ' selected' : ''}`}
                key={item.note.id}
              >
                <button
                  className="view-calendar-agenda-select"
                  onClick={() => onSelect(item.note.id)}
                >
                  <strong>{item.note.title || t('未命名笔记', 'Untitled note')}</strong>
                  <small>{viewStatusLabel(item.note.inbox_state, t)}</small>
                  <TagPills
                    tags={item.metadata.tags}
                    limit={3}
                    moreLabel={(count) => t(`另有 ${count} 个标签`, `${count} more tags`)}
                  />
                </button>
                <button
                  className="icon-button"
                  aria-label={
                    t('打开原文：', 'Open note: ') +
                    (item.note.title || t('未命名笔记', 'Untitled note'))
                  }
                  onClick={() => onOpenNote(item.note.id)}
                >
                  <ExternalLink size={14} />
                </button>
              </div>
            ))}
          </div>
        </section>
      )}
      <div className="view-calendar-weekdays">
        {weekday.map((name) => (
          <span key={name}>{name}</span>
        ))}
      </div>
      <div className="view-calendar-grid">
        {days.map((date) => {
          const key = dayKey(date.toISOString());
          const dayItems = groups.get(key) ?? [];
          return (
            <div
              className={`view-calendar-day${key === today ? ' today' : ''}${date.getMonth() !== Number(month.split('-')[1]) - 1 ? ' outside' : ''}`}
              key={key}
            >
              <span className="view-calendar-number">{date.getDate()}</span>
              {dayItems.slice(0, 3).map((item) => (
                <button
                  className={`view-calendar-note${selectedId === item.note.id ? ' selected' : ''}`}
                  key={item.note.id}
                  onClick={() => onSelect(item.note.id)}
                  title={item.note.title}
                >
                  {item.note.title || t('未命名笔记', 'Untitled note')}
                </button>
              ))}
              {dayItems.length > 3 && (
                <button
                  className="view-calendar-more"
                  aria-expanded={openDay === key}
                  aria-controls="view-calendar-agenda"
                  aria-label={t(
                    `查看当日全部 ${dayItems.length} 篇笔记`,
                    `Show all ${dayItems.length} notes for this day`,
                  )}
                  onClick={() => setOpenDay((current) => (current === key ? null : key))}
                >
                  +{dayItems.length - 3}
                </button>
              )}
              {dayItems.length > 0 && (
                <button
                  className="view-calendar-open"
                  onClick={() => onOpenNote(dayItems[0].note.id)}
                >
                  {t('打开首项', 'Open first')}
                </button>
              )}
            </div>
          );
        })}
      </div>
      {dateProperty && unscheduled.length > 0 && (
        <section
          className="view-calendar-unscheduled"
          aria-label={t('未排期笔记', 'Unscheduled notes')}
        >
          <header>
            <strong>{t('未排期', 'Unscheduled')}</strong>
            <span>{unscheduled.length}</span>
            <small>
              {t(
                `${dateProperty} 缺失或不是有效日期`,
                `${dateProperty} is missing or is not a valid date`,
              )}
            </small>
          </header>
          <div>
            {unscheduled.map((item) => (
              <button
                className={`view-calendar-unscheduled-item${selectedId === item.note.id ? ' selected' : ''}`}
                key={item.note.id}
                aria-pressed={selectedId === item.note.id}
                onClick={() => onSelect(item.note.id)}
              >
                <strong>{item.note.title || t('未命名笔记', 'Untitled note')}</strong>
                <TagPills
                  tags={item.metadata.tags}
                  limit={2}
                  moreLabel={(count) => t(`另有 ${count} 个标签`, `${count} more tags`)}
                />
              </button>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}

function TimelineView({
  view,
  items,
  selectedId,
  onSelect,
  onOpenNote,
}: {
  view: { config: ViewConfig };
  items: ViewNote[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onOpenNote: (id: string) => void;
}) {
  const { t, language } = useI18n();
  const dateProperty = view.config.date_property;
  const { groups, unscheduled } = useMemo(() => {
    const map = new Map<string, ViewNote[]>();
    const unplaced: ViewNote[] = [];
    for (const item of items) {
      const key = itemDayKey(item, dateProperty);
      if (key) map.set(key, [...(map.get(key) ?? []), item]);
      else unplaced.push(item);
    }
    return {
      groups: [...map.entries()].sort(([a], [b]) => b.localeCompare(a)),
      unscheduled: unplaced,
    };
  }, [dateProperty, items]);
  return (
    <div className="view-timeline">
      <div className="view-timeline-source">
        <span>
          {dateProperty
            ? t(`按属性 ${dateProperty}`, `By ${dateProperty}`)
            : t('按最后编辑日期', 'By last edited date')}
        </span>
      </div>
      {groups.map(([day, group]) => (
        <section className="view-timeline-day" key={day}>
          <header>
            <span className="view-timeline-dot" />
            <strong>{day}</strong>
            <small>
              {new Intl.DateTimeFormat(language === 'en' ? 'en-US' : 'zh-CN', {
                weekday: 'long',
              }).format(new Date(`${day}T12:00:00`))}
            </small>
          </header>
          {group.map((item) => (
            <TimelineCard
              item={item}
              view={view}
              dateProperty={dateProperty}
              selectedId={selectedId}
              onSelect={onSelect}
              onOpenNote={onOpenNote}
              key={item.note.id}
            />
          ))}
        </section>
      ))}
      {dateProperty && unscheduled.length > 0 && (
        <section className="view-timeline-day view-timeline-unscheduled">
          <header>
            <span className="view-timeline-dot" />
            <strong>{t('未排期', 'Unscheduled')}</strong>
            <small>
              {t(
                `${dateProperty} 缺失或不是有效日期`,
                `${dateProperty} is missing or is not a valid date`,
              )}
            </small>
          </header>
          {unscheduled.map((item) => (
            <TimelineCard
              item={item}
              view={view}
              dateProperty={dateProperty}
              selectedId={selectedId}
              onSelect={onSelect}
              onOpenNote={onOpenNote}
              key={item.note.id}
            />
          ))}
        </section>
      )}
    </div>
  );
}

function TimelineCard({
  item,
  view,
  dateProperty,
  selectedId,
  onSelect,
  onOpenNote,
}: {
  item: ViewNote;
  view: { config: ViewConfig };
  dateProperty: string | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onOpenNote: (id: string) => void;
}) {
  const { t, language } = useI18n();
  return (
    <article className={`view-timeline-card${selectedId === item.note.id ? ' selected' : ''}`}>
      <button
        className="view-card-select"
        aria-label={t('查看属性：', 'Show properties: ') + item.note.title}
        aria-pressed={selectedId === item.note.id}
        onClick={() => onSelect(item.note.id)}
      />
      <div className="view-card-content">
        <NoteTitle item={item} onOpenNote={onOpenNote} />
        <NotePreview item={item} />
        <small>
          {viewStatusLabel(item.note.inbox_state, t)} · {formatDate(item.note.updated_at, language)}
        </small>
        {(item.metadata.tags.length > 0 || view.config.custom_properties.length > 0) && (
          <div className="view-card-meta">
            {item.metadata.tags.length > 0 && (
              <TagPills
                tags={item.metadata.tags}
                limit={4}
                moreLabel={(count) => t(`另有 ${count} 个标签`, `${count} more tags`)}
              />
            )}
            <CustomPropertySummary
              item={item}
              keys={view.config.custom_properties.filter((key) => key !== dateProperty)}
              language={language}
              t={t}
              limit={3}
            />
          </div>
        )}
      </div>
    </article>
  );
}

export function ViewMetadataEditor({
  item,
  identity,
  onChanged,
  onOpenNote,
  onClose,
}: {
  item: ViewMetadataItem;
  identity: { local_instance_id: string; workspace_id: string };
  onChanged: () => void;
  onOpenNote: (id: string) => void;
  onClose?: () => void;
}) {
  const { t, language } = useI18n();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tagName, setTagName] = useState('');
  async function mutate(command: Parameters<typeof call>[0]) {
    setBusy(true);
    setError(null);
    try {
      await call(command, 'metadata_commit', identity);
      onChanged();
    } catch (failure) {
      setError(message(failure));
    } finally {
      setBusy(false);
    }
  }
  function setInbox(target: InboxState) {
    void mutate({
      kind: 'objects_set_inbox_state',
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          object_id: item.note.id,
          base_row_version: item.note.row_version,
          target,
        },
      },
    });
  }
  function setFlag(kind: 'objects_set_archived' | 'objects_set_search_enabled', value: boolean) {
    void mutate({
      kind,
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          object_id: item.note.id,
          base_row_version: item.note.row_version,
          value,
        },
      },
    });
  }
  function removeTag(tag: Tag) {
    void mutate({
      kind: 'tags_remove',
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          targets: [{ object_id: item.note.id, base_row_version: item.note.row_version }],
          tag_id: tag.id,
        },
      },
    });
  }
  function addTag() {
    const name = tagName.trim();
    if (!name) return;
    void mutate({
      kind: 'tags_assign',
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          targets: [{ object_id: item.note.id, base_row_version: item.note.row_version }],
          tag: { kind: 'name', name },
        },
      },
    }).then(() => setTagName(''));
  }
  const metadata: ObjectMetadata = item.metadata;
  const properties = item.properties;
  return (
    <section className="view-metadata-editor" aria-label={t('笔记属性', 'Note properties')}>
      <header>
        <div>
          <span className="view-eyebrow">{t('视图内属性', 'View properties')}</span>
          <h3>{item.note.title || t('未命名笔记', 'Untitled note')}</h3>
        </div>
        <button className="secondary-button" onClick={() => onOpenNote(item.note.id)}>
          <ExternalLink size={14} />
          {t('回到原文', 'Open original')}
        </button>
        {onClose && (
          <button
            className="icon-button"
            aria-label={t('关闭笔记属性', 'Close note properties')}
            onClick={onClose}
          >
            <X size={15} />
          </button>
        )}
      </header>
      <div className="view-metadata-grid">
        <div className="choice-field">
          {t('整理状态', 'State')}
          <Select
            aria-label={t('整理状态', 'State')}
            value={metadata.inbox_state ?? 'pending'}
            disabled={busy}
            onValueChange={(value) => setInbox(value as InboxState)}
          >
            <option value="pending">{t('待整理', 'Inbox')}</option>
            <option value="snoozed">{t('稍后整理', 'Later')}</option>
            <option value="processed">{t('已整理', 'Organized')}</option>
          </Select>
        </div>
        <label className="view-flag">
          <input
            type="checkbox"
            checked={metadata.archived}
            disabled={busy}
            onChange={(event) => setFlag('objects_set_archived', event.target.checked)}
          />
          {t('已归档', 'Archived')}
        </label>
        <label className="view-flag">
          <input
            type="checkbox"
            checked={metadata.search_enabled}
            disabled={busy}
            onChange={(event) => setFlag('objects_set_search_enabled', event.target.checked)}
          />
          {t('参与搜索', 'Included in search')}
        </label>
      </div>
      <div className="view-tags-editor">
        <div className="view-tags-row">
          <TagIcon size={14} />
          {metadata.tags.map((tag) => (
            <button
              key={tag.id}
              className={`view-tag-chip ${tagToneClass(tag.id)}`}
              disabled={busy}
              onClick={() => removeTag(tag)}
              title={t('移除标签', 'Remove tag')}
            >
              {tag.display_name}
              <Trash2 size={11} />
            </button>
          ))}
          {!metadata.tags.length && <span className="view-muted">{t('没有标签', 'No tags')}</span>}
        </div>
        <div className="view-tag-add">
          <input
            value={tagName}
            disabled={busy}
            placeholder={t('添加标签…', 'Add a tag…')}
            onChange={(event) => setTagName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                event.preventDefault();
                addTag();
              }
            }}
          />
          <button className="secondary-button" disabled={busy || !tagName.trim()} onClick={addTag}>
            {t('添加', 'Add')}
          </button>
        </div>
      </div>
      {properties !== undefined && (
        <div className="view-frontmatter-properties">
          <div className="view-frontmatter-heading">
            <div>
              <strong>{t('Frontmatter 属性', 'Frontmatter properties')}</strong>
              <small>
                {t(
                  '只读投影；请回到原文修改 YAML，这里不会制造第二份属性数据。',
                  'A read-only projection. Edit the YAML in the original note; this panel never creates a second copy of the property data.',
                )}
              </small>
            </div>
            <span>{properties.length}</span>
          </div>
          {item.properties_error && (
            <p className="view-frontmatter-warning" role="alert">
              {t(
                '无法解析当前笔记的 frontmatter。原文未被修改，可回到原文检查 YAML。',
                'The frontmatter in this note could not be parsed. The source was not changed; open the original to check its YAML.',
              )}
            </p>
          )}
          {properties.length > 0 ? (
            <dl className="view-frontmatter-list">
              {properties.map((property) => (
                <div className="view-frontmatter-row" key={property.key}>
                  <dt>{property.key}</dt>
                  <dd>
                    <span
                      title={
                        property.truncated
                          ? t('属性值已截断', 'Property value truncated')
                          : undefined
                      }
                    >
                      {customPropertyText(property, language, t)}
                    </span>
                    <small>{propertyKindLabel(property.kind, t)}</small>
                  </dd>
                </div>
              ))}
            </dl>
          ) : (
            !item.properties_error && (
              <p className="view-muted">
                {t('这篇笔记还没有 frontmatter 属性。', 'This note has no frontmatter properties.')}
              </p>
            )
          )}
          {item.properties_truncated && (
            <p className="view-frontmatter-notice" role="status">
              {t(
                '属性投影已达到数量或长度上限；未显示的内容仍完整保存在原文 YAML 中。',
                'The property projection reached its count or length limit. Any hidden content remains intact in the original YAML.',
              )}
            </p>
          )}
        </div>
      )}
      {error && (
        <p className="view-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

export function ViewPrintHeader({ view }: { view: { name: string; layout: ViewLayout } }) {
  const { t } = useI18n();
  return (
    <div className="view-print-header">
      <strong>{view.name}</strong>
      <span>{layoutLabel(view.layout, t)}</span>
    </div>
  );
}

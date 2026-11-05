import { useEffect, useState } from 'react';
import {
  ArrowLeft,
  ArrowUpRight,
  BookOpen,
  ExternalLink,
  FolderPlus,
  Inbox,
  LoaderCircle,
  MessageSquareQuote,
  Tag,
  Trash2,
} from 'lucide-react';
import type {
  AnnotationSummary,
  BodyReference,
  KnowledgeObject,
  ObjectMetadata,
  OutgoingRelation,
  RelationBacklink,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import {
  SourceCollectionPicker,
  type SourceCollectionChoice,
} from '../sources/SourceCollectionPicker';
import { tagToneClass } from '../ui/tag-color';

type RelatedState = {
  body: BodyReference[];
  relations: RelationBacklink[];
  outgoing: BodyReference[];
  outgoingRelations: OutgoingRelation[];
  annotations: AnnotationSummary[];
  errors: string[];
};

const emptyRelated: RelatedState = {
  body: [],
  relations: [],
  outgoing: [],
  outgoingRelations: [],
  annotations: [],
  errors: [],
};

export function KnowledgeContextPanel({
  identity,
  item,
  onOpenNote,
  onOpenSource,
  onManageCollections,
  onChanged,
  onClose,
}: {
  identity: WorkspaceIdentity;
  item: KnowledgeObject;
  onOpenNote: (id: string) => void;
  onOpenSource: (id: string, versionId?: string, annotationId?: string) => void;
  onManageCollections: () => void;
  onChanged: () => void;
  onClose: () => void;
}) {
  const { t, language } = useI18n();
  const [metadata, setMetadata] = useState<ObjectMetadata>(item.object);
  const [collections, setCollections] = useState(item.collections);
  const [related, setRelated] = useState<RelatedState>(emptyRelated);
  const [relatedLoading, setRelatedLoading] = useState(true);
  const [collectionChoice, setCollectionChoice] = useState<SourceCollectionChoice | null>(null);
  const [tagName, setTagName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [trashOpen, setTrashOpen] = useState(false);
  const objectId = item.object.id;
  const sourceVersionId = item.source?.current_version_id ?? item.annotation?.source_version_id;
  const noteRevisionId = item.note?.revision_id;

  useEffect(() => {
    setMetadata(item.object);
    setCollections(item.collections);
    setCollectionChoice(null);
    setTagName('');
    setError(null);
    setNotice(null);
    setTrashOpen(false);
  }, [item.object.id, item.object.row_version, item.collections]);

  useEffect(() => {
    let live = true;
    setRelatedLoading(true);
    setRelated(emptyRelated);
    const tasks: Array<Promise<Partial<RelatedState>>> = [
      call(
        { kind: 'notes_backlinks', payload: { id: objectId, limit: 20, cursor: null } },
        'body_reference_page',
        identity,
      ).then((page) => ({ body: page.items })),
      call(
        { kind: 'notes_relation_backlinks', payload: { id: objectId, limit: 20, cursor: null } },
        'relation_backlink_page',
        identity,
      ).then((page) => ({ relations: page.items })),
    ];
    if (noteRevisionId) {
      tasks.push(
        call(
          {
            kind: 'notes_references',
            payload: { id: objectId, revision_id: noteRevisionId, limit: 20, cursor: null },
          },
          'body_reference_page',
          identity,
        ).then((page) => ({ outgoing: page.items })),
        call(
          { kind: 'notes_outgoing_relations', payload: { id: objectId } },
          'outgoing_relations',
          identity,
        ).then((items) => ({ outgoingRelations: items })),
      );
    }
    if (item.object.kind === 'source' && sourceVersionId) {
      tasks.push(
        call(
          {
            kind: 'annotations_query',
            payload: { limit: 100, cursor: null, query: item.title },
          },
          'annotation_page',
          identity,
        ).then((page) => ({
          annotations: page.items
            .filter((annotation) => annotation.source_id === objectId)
            .slice(0, 20),
        })),
      );
    }
    void Promise.allSettled(tasks)
      .then((results) => {
        if (!live) return;
        const next: RelatedState = { ...emptyRelated };
        for (const result of results) {
          if (result.status === 'fulfilled') {
            next.body = result.value.body ?? next.body;
            next.relations = result.value.relations ?? next.relations;
            next.outgoing = result.value.outgoing ?? next.outgoing;
            next.outgoingRelations = result.value.outgoingRelations ?? next.outgoingRelations;
            next.annotations = result.value.annotations ?? next.annotations;
          } else {
            next.errors.push(message(result.reason));
          }
        }
        setRelated(next);
      })
      .finally(() => {
        if (live) setRelatedLoading(false);
      });
    return () => {
      live = false;
    };
  }, [identity, objectId, noteRevisionId, sourceVersionId, item.title, item.object.kind]);

  async function openObject(id: string) {
    try {
      const metadata = await call(
        { kind: 'objects_metadata_get', payload: { object_id: id } },
        'object_metadata',
        identity,
      );
      if (metadata.kind === 'source') {
        onOpenSource(id);
      } else if (metadata.kind === 'annotation') {
        const annotation = await call(
          { kind: 'annotations_get', payload: { id } },
          'annotation',
          identity,
        );
        const version = await call(
          { kind: 'sources_version', payload: { source_version_id: annotation.source_version_id } },
          'source_version',
          identity,
        );
        onOpenSource(version.source_id, annotation.source_version_id, annotation.id);
      } else {
        onOpenNote(id);
      }
    } catch (caught) {
      setError(message(caught));
    }
  }

  function applyMetadata(next: ObjectMetadata) {
    setMetadata(next);
    setNotice(
      t(
        '已更新，分类和标签会同步到搜索结果。',
        'Updated. Categories and tags are now part of search context.',
      ),
    );
  }

  async function setInbox(target: 'pending' | 'snoozed' | 'processed') {
    setBusy(true);
    setError(null);
    try {
      const commit = await call(
        {
          kind: 'objects_set_inbox_state',
          payload: {
            command_id: crypto.randomUUID(),
            request: { object_id: objectId, base_row_version: metadata.row_version, target },
          },
        },
        'metadata_commit',
        identity,
      );
      const next = commit.objects.find((value) => value.id === objectId);
      if (next) applyMetadata(next);
      onChanged();
    } catch (caught) {
      setError(message(caught));
    } finally {
      setBusy(false);
    }
  }

  async function addTag() {
    const name = tagName.trim();
    if (!name) return;
    setBusy(true);
    setError(null);
    try {
      const commit = await call(
        {
          kind: 'tags_assign',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              targets: [{ object_id: objectId, base_row_version: metadata.row_version }],
              tag: { kind: 'name', name },
            },
          },
        },
        'metadata_commit',
        identity,
      );
      const next = commit.objects.find((value) => value.id === objectId);
      if (next) applyMetadata(next);
      setTagName('');
      onChanged();
    } catch (caught) {
      setError(message(caught));
    } finally {
      setBusy(false);
    }
  }

  async function addToCategory() {
    if (!collectionChoice) return;
    setBusy(true);
    setError(null);
    let added = false;
    let categoryName = collectionChoice.name;
    try {
      const collection = await call(
        { kind: 'collections_get', payload: { collection_id: collectionChoice.id } },
        'collection',
        identity,
      );
      categoryName = collection.name;
      await call(
        {
          kind: 'collections_add_members',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              collection_id: collection.id,
              base_row_version: collection.metadata.row_version,
              member_ids: [objectId],
            },
          },
        },
        'collection_commit',
        identity,
      );
      added = true;
      setCollections((current) =>
        current.some((value) => value.id === collection.id)
          ? current
          : [...current, { id: collection.id, name: collection.name }],
      );
      setCollectionChoice(null);
      const current = await call(
        { kind: 'objects_metadata_get', payload: { object_id: objectId } },
        'object_metadata',
        identity,
      );
      if (current.inbox_state === 'processed') {
        setMetadata(current);
      } else {
        const commit = await call(
          {
            kind: 'objects_set_inbox_state',
            payload: {
              command_id: crypto.randomUUID(),
              request: {
                object_id: objectId,
                base_row_version: current.row_version,
                target: 'processed',
              },
            },
          },
          'metadata_commit',
          identity,
        );
        const updated = commit.objects.find((value) => value.id === objectId);
        if (!updated)
          throw new Error(t('整理状态没有返回对象结果。', 'No object result was returned.'));
        setMetadata(updated);
      }
      setNotice(
        t(
          `已加入分类「${categoryName}」并标记为已整理。`,
          `Added to “${categoryName}” and marked organized.`,
        ),
      );
      onChanged();
    } catch (caught) {
      if (added) {
        setError(
          t(
            `已加入分类「${categoryName}」，但整理状态未更新：${message(caught)}`,
            `Added to “${categoryName}”, but the organization state was not updated: ${message(caught)}`,
          ),
        );
        onChanged();
      } else {
        setError(message(caught));
      }
    } finally {
      setBusy(false);
    }
  }

  async function trash() {
    setBusy(true);
    setError(null);
    try {
      await call(
        {
          kind: 'objects_set_trashed',
          payload: {
            command_id: crypto.randomUUID(),
            request: { object_id: objectId, base_row_version: metadata.row_version, value: true },
          },
        },
        'metadata_commit',
        identity,
      );
      setTrashOpen(false);
      onChanged();
      onClose();
    } catch (caught) {
      setError(message(caught));
    } finally {
      setBusy(false);
    }
  }

  const statusLabel =
    metadata.inbox_state === 'processed'
      ? t('已整理', 'Organized')
      : metadata.inbox_state === 'snoozed'
        ? t('稍后整理', 'Later')
        : t('待整理', 'Inbox');
  const objectType =
    item.object.kind === 'source'
      ? t('资料', 'Source')
      : item.object.kind === 'annotation'
        ? t('摘录', 'Excerpt')
        : item.object_type
          ? item.object_type
          : t('笔记', 'Note');
  const relatedObjectIds = new Set<string>();
  for (const value of related.body) relatedObjectIds.add(value.source_id);
  for (const value of related.relations) relatedObjectIds.add(value.source_id);
  for (const value of related.outgoing) relatedObjectIds.add(value.target_id);
  for (const value of related.outgoingRelations) relatedObjectIds.add(value.target_id);
  relatedObjectIds.delete(item.object.id);
  const relatedCount = relatedObjectIds.size + related.annotations.length;

  return (
    <article
      className="knowledge-context knowledge-context-page"
      aria-label={t('对象详情', 'Object details')}
    >
      <header className="knowledge-context-heading">
        <button type="button" className="text-button knowledge-context-back" onClick={onClose}>
          <ArrowLeft size={15} />
          {t('返回知识索引', 'Back to knowledge index')}
        </button>
        <div className="knowledge-context-kind">
          {item.object.kind === 'source' ? (
            <BookOpen size={16} />
          ) : item.object.kind === 'annotation' ? (
            <MessageSquareQuote size={16} />
          ) : (
            <ArrowUpRight size={16} />
          )}
          <span>{objectType}</span>
        </div>
      </header>
      <div className="knowledge-context-title-row">
        <h2>{item.title || t('未命名', 'Untitled')}</h2>
        <span className={`knowledge-status-chip is-${metadata.inbox_state ?? 'pending'}`}>
          {statusLabel}
        </span>
      </div>
      <p className="knowledge-context-preview">{item.preview || t('暂无摘要', 'No preview yet')}</p>
      <div className="knowledge-context-actions">
        <button
          type="button"
          className="primary-button"
          onClick={() => {
            if (item.object.kind === 'source') onOpenSource(objectId, sourceVersionId);
            else if (item.object.kind === 'annotation' && item.annotation)
              onOpenSource(
                item.annotation.source_id,
                item.annotation.source_version_id,
                item.annotation.id,
              );
            else onOpenNote(objectId);
          }}
        >
          <ExternalLink size={14} />
          {item.object.kind === 'source'
            ? t('打开资料', 'Open source')
            : item.object.kind === 'annotation'
              ? t('回到原文摘录', 'Open excerpt in source')
              : t('打开笔记', 'Open note')}
        </button>
        <button
          type="button"
          className="secondary-button"
          disabled={busy}
          onClick={() => setTrashOpen(true)}
        >
          <Trash2 size={14} />
          {t('回收', 'Trash')}
        </button>
      </div>
      {error && (
        <p className="knowledge-context-error" role="alert">
          {error}
        </p>
      )}
      {notice && (
        <p className="knowledge-context-notice" role="status">
          {notice}
        </p>
      )}

      {item.annotation && (
        <section
          className="knowledge-context-section knowledge-excerpt-context"
          aria-label={t('原文摘录', 'Source excerpt')}
        >
          <div className="knowledge-context-section-title">
            <span>
              <MessageSquareQuote size={13} />
              {t('原文摘录', 'Source excerpt')}
            </span>
            <small>{item.annotation.source_title}</small>
          </div>
          <blockquote>{item.annotation.quote_preview}</blockquote>
          {item.annotation.comment_preview && (
            <p className="knowledge-excerpt-comment">{item.annotation.comment_preview}</p>
          )}
          <small className="knowledge-context-muted">
            {item.annotation.page_index != null
              ? t(
                  `第 ${item.annotation.page_index + 1} 页`,
                  `Page ${item.annotation.page_index + 1}`,
                )
              : t('文本块定位', 'Text block locator')}
          </small>
        </section>
      )}

      <section className="knowledge-context-section">
        <div className="knowledge-context-section-title">
          <span>{t('分类', 'Categories')}</span>
          <button type="button" className="text-button" onClick={onManageCollections}>
            <FolderPlus size={13} />
            {t('管理', 'Manage')}
          </button>
        </div>
        <div className="knowledge-context-chips">
          {collections.length ? (
            collections.map((collection) => (
              <span className="knowledge-chip" key={collection.id}>
                {collection.name}
              </span>
            ))
          ) : (
            <span className="knowledge-context-muted">{t('尚未归入分类', 'Not filed yet')}</span>
          )}
        </div>
        <div className="knowledge-context-inline-form">
          <SourceCollectionPicker
            identity={identity}
            value={collectionChoice}
            onChange={setCollectionChoice}
            disabled={busy}
            label={t('加入分类', 'Add to category')}
            emptyLabel={t('选择分类…', 'Choose a category…')}
          />
          <button
            type="button"
            className="secondary-button"
            disabled={busy || !collectionChoice}
            onClick={() => void addToCategory()}
          >
            {t('加入', 'Add')}
          </button>
        </div>
      </section>

      <section className="knowledge-context-section">
        <div className="knowledge-context-section-title">
          <span>{t('标签', 'Tags')}</span>
        </div>
        <div className="knowledge-context-chips">
          {metadata.tags.length ? (
            metadata.tags.map((tag) => (
              <span className={`tag-chip ${tagToneClass(tag.id)}`} key={tag.id}>
                <Tag size={11} />
                {tag.display_name}
              </span>
            ))
          ) : (
            <span className="knowledge-context-muted">{t('还没有标签', 'No tags yet')}</span>
          )}
        </div>
        <form
          className="knowledge-context-inline-form"
          onSubmit={(event) => {
            event.preventDefault();
            void addTag();
          }}
        >
          <input
            value={tagName}
            onChange={(event) => setTagName(event.target.value)}
            placeholder={t('输入标签名…', 'Tag name…')}
            aria-label={t('添加标签', 'Add tag')}
            disabled={busy}
          />
          <button className="secondary-button" disabled={busy || !tagName.trim()}>
            {t('添加', 'Add')}
          </button>
        </form>
      </section>

      <section className="knowledge-context-section">
        <div className="knowledge-context-section-title">
          <span>{t('整理状态', 'Organization')}</span>
        </div>
        <div
          className="knowledge-context-segments"
          role="group"
          aria-label={t('整理状态', 'Organization status')}
        >
          {(['pending', 'snoozed', 'processed'] as const).map((target) => (
            <button
              key={target}
              type="button"
              className={metadata.inbox_state === target ? 'active' : ''}
              aria-pressed={metadata.inbox_state === target}
              disabled={busy}
              onClick={() => void setInbox(target)}
            >
              {target === 'pending'
                ? t('待整理', 'Inbox')
                : target === 'snoozed'
                  ? t('稍后', 'Later')
                  : t('已整理', 'Organized')}
            </button>
          ))}
        </div>
      </section>

      <section className="knowledge-context-section">
        <div className="knowledge-context-section-title">
          <span>{t('关系与引用', 'Relations and references')}</span>
          {relatedCount > 0 && <small>{relatedCount}</small>}
        </div>
        {relatedLoading && (
          <p className="knowledge-context-muted" role="status">
            <LoaderCircle size={14} className="spin" />
            {t('正在读取关系…', 'Loading relations…')}
          </p>
        )}
        {!relatedLoading && relatedCount === 0 && !related.errors.length && (
          <p className="knowledge-context-muted">
            {t(
              '还没有可解释的关系。正文引用或属性 relation 会显示在这里。',
              'No explainable relations yet. Body references and frontmatter relations appear here.',
            )}
          </p>
        )}
        {!relatedLoading && related.errors.length > 0 && (
          <p className="knowledge-context-error">
            {t('部分关系未能读取：', 'Some relations could not be read: ')}
            {related.errors[0]}
          </p>
        )}
        {related.body.length > 0 && (
          <div className="knowledge-related-group">
            <h3>{t('被这些笔记引用', 'Referenced by')}</h3>
            {related.body.map((value) => (
              <button
                type="button"
                className="knowledge-related-row"
                key={`${value.source_id}/${value.occurrence_id}`}
                onClick={() => onOpenNote(value.source_id)}
              >
                <span>{value.source_title}</span>
                <small>{value.context}</small>
              </button>
            ))}
          </div>
        )}
        {related.relations.length > 0 && (
          <div className="knowledge-related-group">
            <h3>{t('属性关系', 'Property relations')}</h3>
            {related.relations.map((value) => (
              <button
                type="button"
                className="knowledge-related-row"
                key={`${value.source_id}/${value.field_key}`}
                onClick={() => onOpenNote(value.source_id)}
              >
                <span>{value.source_title}</span>
                <small>{value.context}</small>
              </button>
            ))}
          </div>
        )}
        {related.outgoing.length > 0 && (
          <div className="knowledge-related-group">
            <h3>{t('这篇笔记引用', 'References from this note')}</h3>
            {related.outgoing.map((value) => (
              <button
                type="button"
                className="knowledge-related-row"
                key={`${value.target_id}/${value.occurrence_id}`}
                onClick={() => void openObject(value.target_id)}
              >
                <span>{value.label || value.target_id}</span>
                <small>{value.context}</small>
              </button>
            ))}
          </div>
        )}
        {related.outgoingRelations.length > 0 && (
          <div className="knowledge-related-group">
            <h3>{t('本笔记的属性关系', 'Property relations from this note')}</h3>
            {related.outgoingRelations.map((value) => (
              <button
                type="button"
                className="knowledge-related-row"
                key={`${value.field_key}/${value.target_id}`}
                onClick={() => void openObject(value.target_id)}
              >
                <span>{value.relation_title}</span>
                <small>{value.context}</small>
              </button>
            ))}
          </div>
        )}
        {related.annotations.length > 0 && (
          <div className="knowledge-related-group">
            <h3>
              <MessageSquareQuote size={13} />
              {t('摘录', 'Excerpts')}
            </h3>
            {related.annotations.map((value) => (
              <button
                type="button"
                className="knowledge-related-row"
                key={value.id}
                onClick={() => onOpenSource(item.object.id, sourceVersionId, value.id)}
              >
                <span>
                  {value.quote_preview ||
                    value.comment_preview ||
                    t('无文字摘录', 'Excerpt without quote')}
                </span>
                <small>
                  {value.page_index != null
                    ? t(`第 ${value.page_index + 1} 页`, `Page ${value.page_index + 1}`)
                    : t('原文定位', 'Source locator')}
                </small>
              </button>
            ))}
          </div>
        )}
      </section>

      <footer className="knowledge-context-footer">
        <span>
          {t('更新于', 'Updated')}{' '}
          {new Date(item.updated_at).toLocaleString(language === 'en' ? 'en-US' : 'zh-CN')}
        </span>
        <Inbox size={13} aria-hidden="true" />
      </footer>

      {trashOpen && (
        <ActionConfirmDialog
          title={t('回收这份对象？', 'Move this object to trash?')}
          description={t(
            '对象和它的整理信息会进入回收站，之后仍可恢复。',
            'The object and its organization metadata move to trash and can still be restored.',
          )}
          confirmLabel={t('移入回收站', 'Move to trash')}
          tone="danger"
          onCancel={() => setTrashOpen(false)}
          onConfirm={() => void trash()}
        />
      )}
    </article>
  );
}

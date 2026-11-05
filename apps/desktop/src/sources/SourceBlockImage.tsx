import { useEffect, useRef, useState } from 'react';
import type { SourceBlockImage as SourceBlockImageRef, WorkspaceIdentity } from '@pkms/contracts';
import { ImageOff, Image as ImageIcon, RotateCw } from 'lucide-react';
import { call, message } from '../api';
import { useI18n } from '../i18n';

export function SourceBlockImage({
  identity,
  versionId,
  parseId,
  image,
}: {
  identity: WorkspaceIdentity;
  versionId: string;
  parseId: string;
  image: SourceBlockImageRef;
}) {
  const { t } = useI18n();
  const localInstanceId = identity.local_instance_id;
  const workspaceId = identity.workspace_id;
  const host = useRef<HTMLDivElement>(null);
  const [requested, setRequested] = useState(false);
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);

  useEffect(() => {
    if (requested) return;
    const node = host.current;
    if (!node || typeof IntersectionObserver === 'undefined') {
      setRequested(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setRequested(true);
          observer.disconnect();
        }
      },
      { rootMargin: '600px 0px' },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [requested, retry]);

  useEffect(() => {
    if (!requested) return;
    let live = true;
    setError(null);
    setSrc(null);
    void call(
      {
        kind: 'sources_image',
        payload: {
          source_version_id: versionId,
          parse_id: parseId,
          blob_hash: image.blob_hash,
        },
      },
      'source_image',
      identity,
      30_000,
    )
      .then((result) => {
        if (!live) return;
        if (
          result.source_version_id !== versionId ||
          result.parse_id !== parseId ||
          result.blob_hash !== image.blob_hash ||
          result.media_type !== image.media_type ||
          !/^image\/(?:png|jpeg|webp)$/i.test(result.media_type) ||
          !result.data_base64 ||
          result.data_base64.length > 20 * 1024 * 1024
        ) {
          setError(
            t(
              '图片响应与当前资料不一致，已停止显示。',
              'The image response did not match this document.',
            ),
          );
          return;
        }
        setSrc(`data:${result.media_type};base64,${result.data_base64}`);
      })
      .catch((failure) => {
        if (live) setError(message(failure));
      });
    return () => {
      live = false;
    };
  }, [
    localInstanceId,
    workspaceId,
    versionId,
    parseId,
    image.blob_hash,
    image.media_type,
    requested,
    retry,
  ]);

  return (
    <div
      ref={host}
      className="source-inline-image-host"
      data-image-state={error ? 'error' : src ? 'ready' : requested ? 'loading' : 'idle'}
    >
      {src ? (
        <img
          src={src}
          alt={image.alt || t('书内图片', 'Book illustration')}
          decoding="async"
          onError={() => {
            setSrc(null);
            setError(t('图片无法解码或显示。', 'The image could not be decoded or displayed.'));
          }}
        />
      ) : error ? (
        <div className="source-inline-image-status" role="status">
          <ImageOff size={20} aria-hidden="true" />
          <span>{error}</span>
          <button
            type="button"
            className="secondary-button"
            onClick={() => {
              setError(null);
              setRetry((value) => value + 1);
            }}
          >
            <RotateCw size={13} aria-hidden="true" />
            {t('重试图片', 'Retry image')}
          </button>
        </div>
      ) : (
        <div className="source-inline-image-status" role="status">
          <ImageIcon size={20} aria-hidden="true" />
          <span>
            {requested
              ? t('正在读取图片…', 'Loading image…')
              : t('图片将在滚动到附近时加载', 'Image loads when nearby')}
          </span>
        </div>
      )}
      {image.alt && <p className="source-inline-image-caption">{image.alt}</p>}
    </div>
  );
}

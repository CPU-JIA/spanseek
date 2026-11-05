import { useEffect, useState } from 'react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call, imageUrl, message } from '../api';
import { useI18n } from '../i18n';

export function DocumentImage({
  identity,
  revision,
  hash,
  importCommand,
  alt,
}: {
  identity: WorkspaceIdentity;
  revision: string;
  hash: string;
  importCommand?: string;
  alt: string;
}) {
  const { t } = useI18n();
  const key = `${identity.local_instance_id}/${identity.workspace_id}/${revision}/${hash}/${importCommand ?? ''}`;
  const [preview, setPreview] = useState<{ key: string; src: string; error: string | null } | null>(
    null,
  );
  useEffect(() => {
    if (!importCommand) return;
    let live = true;
    void call(
      { kind: 'attachment_resume', payload: { command_id: importCommand } },
      'attachment',
      identity,
    )
      .then((result) => {
        if (live)
          setPreview({
            key,
            src: imageUrl(identity, { token: result.preview_token }),
            error: null,
          });
      })
      .catch((cause) => {
        if (live) setPreview({ key, src: '', error: message(cause) });
      });
    return () => {
      live = false;
    };
  }, [key]);
  const current = preview?.key === key ? preview : null;
  if (current?.error) return <p role="alert">{current.error}</p>;
  const src = importCommand ? current?.src : imageUrl(identity, { revision, hash });
  return src ? (
    <img
      alt={alt}
      src={src}
      loading="lazy"
      onError={() =>
        setPreview({
          key,
          src: '',
          error: t(
            '图片读取失败，原有内容仍保留。',
            'The image could not be read. Its content is retained.',
          ),
        })
      }
    />
  ) : (
    <p role="status">{t('正在读取图片…', 'Loading image…')}</p>
  );
}

import { useEffect, useState } from 'react';
import { message } from '../api';
import type { MarkdownDocument } from './markdown-document';
import { requestMarkdown } from './markdown-request';
import type { SearchEvidence } from '@pkms/contracts';

type Result = {
  source: string;
  allowFrontmatter: boolean;
  value: MarkdownDocument | null;
  error: string | null;
  materials?: readonly SearchEvidence[];
};

/** Parse off the UI thread; completed history is stable while another message streams. */
export function useMarkdownDocument(
  source: string,
  allowFrontmatter = true,
  enabled = true,
  materials?: readonly SearchEvidence[],
) {
  const [result, setResult] = useState<Result | null>(null);
  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    void requestMarkdown(
      materials
        ? { kind: 'task-preview', source, materials }
        : { kind: 'parse', source, allowFrontmatter },
      controller.signal,
    ).then(
      (value) => {
        if (!controller.signal.aborted)
          setResult({ source, allowFrontmatter, materials, value, error: null });
      },
      (error) => {
        if (!controller.signal.aborted)
          setResult({ source, allowFrontmatter, materials, value: null, error: message(error) });
      },
    );
    return () => controller.abort();
  }, [source, allowFrontmatter, enabled, materials]);
  const matches =
    enabled &&
    result?.source === source &&
    result.allowFrontmatter === allowFrontmatter &&
    result.materials === materials;
  return {
    value: matches ? result.value : null,
    error: matches ? result.error : null,
    pending: enabled && !matches,
  };
}

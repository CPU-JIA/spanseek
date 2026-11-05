import type { Tag } from '@pkms/contracts';
import { tagToneClass } from './tag-color';

export function TagPills({
  tags,
  limit,
  moreLabel,
}: {
  tags: readonly Tag[];
  limit?: number;
  moreLabel?: (count: number) => string;
}) {
  if (!tags.length) return null;
  const visible = limit === undefined ? tags : tags.slice(0, limit);
  const remaining = tags.length - visible.length;
  return (
    <span className="tag-pills">
      {visible.map((tag) => (
        <span
          className={`tag-inline ${tagToneClass(tag.id)}`}
          key={tag.id}
          title={`#${tag.display_name}`}
        >
          {tag.display_name}
        </span>
      ))}
      {remaining > 0 && (
        <span
          className="tag-inline tag-inline-more"
          aria-label={moreLabel?.(remaining)}
          title={tags
            .slice(visible.length)
            .map((tag) => `#${tag.display_name}`)
            .join(' ')}
        >
          +{remaining}
        </span>
      )}
    </span>
  );
}

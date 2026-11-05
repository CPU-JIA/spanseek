import { useEffect, useId, useState } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import { useI18n } from '../i18n';
import './hidden-content.css';

/** Visibility is local UI state. The source and the existing %% export contract are untouched. */
export function HiddenContent({ source, block = false }: { source: string; block?: boolean }) {
  const { t } = useI18n();
  const [revealed, setRevealed] = useState(false);
  const id = useId();
  useEffect(() => setRevealed(false), [source]);
  const Icon = revealed ? EyeOff : Eye;
  return (
    <button
      type="button"
      className={'rich-hidden-content' + (block ? ' is-block' : '')}
      data-revealed={revealed}
      aria-pressed={revealed}
      aria-label={
        revealed
          ? t('重新模糊隐藏内容', 'Conceal hidden content')
          : t('显示隐藏内容', 'Reveal hidden content')
      }
      aria-describedby={revealed ? id : undefined}
      onMouseDown={(event) => {
        if (!revealed) event.preventDefault();
      }}
      onClick={(event) => {
        event.stopPropagation();
        const selection = window.getSelection();
        if (
          event.detail > 0 &&
          revealed &&
          selection &&
          !selection.isCollapsed &&
          event.currentTarget.contains(selection.anchorNode) &&
          event.currentTarget.contains(selection.focusNode)
        )
          return;
        setRevealed((value) => !value);
      }}
    >
      <span className="rich-hidden-state" aria-hidden="true">
        <Icon size={13} />
        {block && (
          <span>
            {revealed
              ? t('已显示 · 点击模糊', 'Visible · click to conceal')
              : t('隐藏内容 · 点击显示', 'Hidden content · click to reveal')}
          </span>
        )}
      </span>
      <span id={id} className="rich-hidden-value" aria-hidden={!revealed}>
        {source || t('空隐藏内容', 'Empty hidden content')}
      </span>
    </button>
  );
}

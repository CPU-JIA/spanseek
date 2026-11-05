import './brand-mark.css';

export function BrandMark({ size = 32, className = '' }: { size?: number; className?: string }) {
  return (
    <img
      className={['brand-mark-image', className].filter(Boolean).join(' ')}
      src={`${import.meta.env.BASE_URL}icon.svg`}
      width={size}
      height={size}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}

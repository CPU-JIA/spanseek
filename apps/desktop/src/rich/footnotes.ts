/** Match the persisted footnote identity rule while retaining original display labels. */
export function footnoteKey(label: string): string {
  return label
    .split(/\p{White_Space}+/u)
    .filter(Boolean)
    .join(' ')
    .toLowerCase();
}

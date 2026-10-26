/** A key as a label a person reads: words split at `_` and at a
 *  lower-to-upper change, the first capitalised (`committed_at` →
 *  "Committed at"). Where a page gives no `label`. */
export function humanize(key: string): string {
  const words = key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .split(/[_\s]+/)
    .filter(Boolean)
    .map((w) => w.toLowerCase())
    .join(" ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

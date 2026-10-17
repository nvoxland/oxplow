/**
 * How a saved conversation's size and last activity read in a list: the
 * conversation picker (`conversationRows.ts`) and the New Session page's
 * earlier sessions (`earlierSessionRows.ts`).
 */

/** `1.2 MB`, `34 KB`, `512 B`; null when the size is unknown. */
export function formatSize(bytes: number | null): string | null {
  if (bytes === null) return null;
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** `YYYY-MM-DD HH:MM`, local; null when absent or not a time. */
export function formatWhen(at: string | null): string | null {
  if (!at) return null;
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return null;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

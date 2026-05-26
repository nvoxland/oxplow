/**
 * Two-letter glyph for a title, used by the far-left `Navigator` stream/
 * thread strip and the Terminal-page tab strip so both abbreviate the
 * same way.
 *
 * Prefers the initials of the first two whitespace-separated words
 * ("Git Dashboard" → "GD"); falls back to the first two characters of a
 * single word ("oxplow" → "Ox"). One-character titles uppercase the sole
 * character; an empty/whitespace title yields "?".
 */
export function titleInitials(s: string): string {
  const trimmed = s.trim();
  if (!trimmed) return "?";
  const parts = trimmed.split(/\s+/);
  if (parts.length >= 2 && parts[1].length > 0) {
    return (parts[0].charAt(0) + parts[1].charAt(0)).toUpperCase();
  }
  if (trimmed.length >= 2) {
    return trimmed.charAt(0).toUpperCase() + trimmed.charAt(1).toLowerCase();
  }
  return trimmed.charAt(0).toUpperCase();
}

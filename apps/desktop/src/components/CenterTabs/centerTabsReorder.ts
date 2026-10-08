// Pure reorder math for CenterTabs, extracted so it's unit-testable
// without a DOM. The strip is `[...pinned, ...reorderable]` where pinned
// tabs (the agent sessions') stay at the front.

/// Number of leading pinned tabs. New promotions land right after this
/// run, and a drag never lands inside it.
export function leadingPinnedCount(pinnedFlags: boolean[]): number {
  let n = 0;
  while (n < pinnedFlags.length && pinnedFlags[n]) n++;
  return n;
}

/// Move `id` to the slot right after the leading pinned run, returning
/// the new id order. Returns the same array (no-op) when `id` is absent
/// or already in that slot.
export function reorderToAfterPinned(
  ids: string[],
  pinnedCount: number,
  id: string,
): string[] {
  const fromIdx = ids.indexOf(id);
  const insertIdx = pinnedCount;
  if (fromIdx < 0 || fromIdx === insertIdx) return ids;
  const next = ids.slice();
  const [moved] = next.splice(fromIdx, 1);
  // Splicing the source out shifts later indices left by one.
  const adjusted = fromIdx < insertIdx ? insertIdx - 1 : insertIdx;
  next.splice(adjusted, 0, moved);
  return next;
}

/// Move `id` so it occupies `desiredIndex` in the ORIGINAL array order
/// (i.e. the insertion point chosen before removing the source).
/// Handles the left-shift that removing an earlier source causes, so the
/// drop lands exactly where the insertion line was shown. No-op when the
/// move wouldn't change anything.
export function moveToIndex(ids: string[], id: string, desiredIndex: number): string[] {
  const fromIdx = ids.indexOf(id);
  if (fromIdx < 0) return ids;
  const clamped = Math.max(0, Math.min(desiredIndex, ids.length));
  const adjusted = fromIdx < clamped ? clamped - 1 : clamped;
  if (adjusted === fromIdx) return ids;
  const next = ids.slice();
  const [moved] = next.splice(fromIdx, 1);
  next.splice(adjusted, 0, moved);
  return next;
}

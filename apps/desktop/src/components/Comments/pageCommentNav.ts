import type { CommentThread } from "../../tauri-bridge/generated/bindings.js";

// Pure helpers for stepping between a page's comments, shared by the
// nav-bar CommentNavigator and the per-thread CommentPopover prev/next.

/// Split a page's comment threads into the ones we can jump to (anchored
/// — they have an inline highlight to scroll to) and the ones we can
/// only list (orphaned — anchor lost).
export function partitionPageComments(threads: CommentThread[]): {
  jumpable: CommentThread[];
  orphaned: CommentThread[];
  total: number;
} {
  const jumpable: CommentThread[] = [];
  const orphaned: CommentThread[] = [];
  for (const t of threads) {
    if (t.comment.orphaned) orphaned.push(t);
    else jumpable.push(t);
  }
  return { jumpable, orphaned, total: threads.length };
}

/// The id of the comment `dir` steps away from `currentId` among the
/// jumpable (anchored) comments, cycling. Returns null when there's
/// nowhere to step (zero or one jumpable comment). When the current
/// comment isn't jumpable (e.g. orphaned), steps from the list edge so
/// prev/next still reach the anchored comments.
export function stepComment(
  threads: CommentThread[],
  currentId: string,
  dir: -1 | 1,
): string | null {
  const { jumpable } = partitionPageComments(threads);
  if (jumpable.length === 0) return null;
  const idx = jumpable.findIndex((t) => t.comment.id === currentId);
  if (idx < 0) {
    // Current isn't in the jumpable set — enter from the matching edge.
    const entry = dir === 1 ? 0 : jumpable.length - 1;
    return jumpable[entry].comment.id;
  }
  if (jumpable.length === 1) return null;
  const next = (idx + dir + jumpable.length) % jumpable.length;
  return jumpable[next].comment.id;
}

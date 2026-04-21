import { resolve, sep } from "node:path";

/**
 * Returns true if `absOrRel`, resolved against `worktreeRoot`, lives inside
 * (or equals) the worktree directory. Shared between the runtime's
 * hook-path filter and the write guard so both use identical containment
 * semantics.
 */
export function isInsideWorktree(absOrRel: string, worktreeRoot: string): boolean {
  const normalizedRoot = resolve(worktreeRoot);
  const candidate = resolve(normalizedRoot, absOrRel);
  if (candidate === normalizedRoot) return true;
  return candidate.startsWith(normalizedRoot + sep);
}

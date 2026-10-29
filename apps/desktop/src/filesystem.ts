/**
 * Where a file's bytes come from — the frontend side of Rust's
 * `oxplow_domain::filesystem` (`.context/filesystems.md`). A filesystem
 * is named by an id (`oxplow:local`, `acme:s3`); a reader that names none
 * means the local one, the stream's worktree. A revision is the
 * filesystem's own word (`revision.ts` is the local one's), so it is
 * never read without the filesystem it belongs to.
 */
import { shortRevisionLabel, WORKING } from "./revision.js";

/** The stream's worktree as files: what a file URI means without a `filesystem`. */
export const LOCAL_FS = "oxplow:local";

/** `filesystem` (none: the local one) and `rev` as the short label a
 *  file's tab or header shows: a local revision as `shortRevisionLabel`
 *  has it, another filesystem's revision shortened when it is a hash, and
 *  the filesystem's id when it names no revision. */
export function fsRevLabel(filesystem: string | null | undefined, rev: string | null | undefined): string {
  if (!filesystem || filesystem === LOCAL_FS) return shortRevisionLabel(rev ?? WORKING);
  if (!rev) return filesystem;
  return /^[0-9a-f]{13,64}$/i.test(rev) ? rev.slice(0, 7) : rev;
}

/**
 * Where a file's bytes come from — the frontend side of Rust's
 * `oxplow_domain::filesystem` (`.context/filesystems.md`). A filesystem
 * is named by an id (`oxplow:local`, `acme:s3`); a reader that names none
 * means the local one, the stream's worktree. A revision is the
 * filesystem's own word (`revision.ts` is the local one's), so it is
 * never read without the filesystem it belongs to.
 */
import type { FsStat } from "./tauri-bridge/generated/bindings.js";
import { shortRevisionLabel, WORKING } from "./revision.js";

/** The stream's worktree as files: what a file URI means without a `filesystem`. */
export const LOCAL_FS = "oxplow:local";

/** Scratch files oxplow keeps for a while: a shown view is a page file here. */
export const TEMP_FS = "oxplow:temp";

/** `filesystem` (none: the local one) and `rev` as the short label a
 *  file's tab or header shows: a local revision as `shortRevisionLabel`
 *  has it, another filesystem's revision shortened when it is a hash, and
 *  the filesystem's id when it names no revision. */
export function fsRevLabel(filesystem: string | null | undefined, rev: string | null | undefined): string {
  if (!filesystem || filesystem === LOCAL_FS) return shortRevisionLabel(rev ?? WORKING);
  if (!rev) return filesystem;
  return /^[0-9a-f]{13,64}$/i.test(rev) ? rev.slice(0, 7) : rev;
}

/** The largest file a viewer loads into its editor: 5 MiB. A bigger one
 *  shows [`unshownReason`] instead. */
export const MAX_SHOWN_BYTES = 5 * 1024 * 1024;

/** Why a viewer doesn't load the file `stat` describes, or null when it
 *  does: a binary file or one over [`MAX_SHOWN_BYTES`] says what it is and
 *  how many bytes it holds. Read before the file is, so neither is loaded. */
export function unshownReason(stat: FsStat): string | null {
  if (stat.kind === "dir") return "a folder, not a file";
  const bytes = stat.size ?? 0;
  if (stat.isText === false) return `binary file, ${bytes} bytes`;
  if (bytes > MAX_SHOWN_BYTES) return `too large to show, ${bytes} bytes`;
  return null;
}

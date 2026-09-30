/**
 * Which version of a file or tree — the frontend side of Rust's
 * `oxplow_domain::vcs::Revision` (`.context/vcs.md`). One string, the
 * same on the wire, in tab ids and in the ref grammar's `@rev` slot:
 *
 * - `working` — the files on disk;
 * - `snap:<id>` — a local-history snapshot;
 * - `<vcs>:<rev>` — a VCS revision (`git:HEAD`, `git:4c44d4…`).
 *
 * Every read, diff and comparison names one explicitly: there is no
 * implicit "the working tree" default.
 */
export type Revision = string;

export const WORKING: Revision = "working";

/** A git revision: anything `git rev-parse` understands. */
export function gitRevision(rev: string): Revision {
  return `git:${rev}`;
}

export function snapshotRevision(id: number | string): Revision {
  return `snap:${id}`;
}

/** The snapshot id `rev` names, or null. */
export function snapshotIdOf(rev: Revision): number | null {
  const m = /^snap:(\d+)$/.exec(rev);
  return m ? Number(m[1]) : null;
}

/** `raw` as a revision, or null when it isn't one. */
export function parseRevision(raw: unknown): Revision | null {
  if (typeof raw !== "string") return null;
  if (raw === WORKING) return raw;
  if (/^snap:\d+$/.test(raw)) return raw;
  if (/^[a-z][a-z0-9_]*:.+$/.test(raw) && !raw.startsWith("snap:")) return raw;
  return null;
}

/** The ref grammar's `@rev` slot (`.context/refs.md`): null for the
 *  working tree. */
export function revisionSlot(rev: Revision): string | null {
  return rev === WORKING ? null : rev;
}

/** Inverse of `revisionSlot`; a slot that isn't a revision reads as the
 *  working tree rather than failing the whole ref. */
export function revisionFromSlot(slot: string | null): Revision {
  return slot === null ? WORKING : (parseRevision(slot) ?? WORKING);
}

/** True when `s` looks like a git commit sha (7–40 hex chars) — tells a
 *  real Change-Analysis commit target from a synthetic diff-view key
 *  like `endpoints:…` / `effort:…`. */
export function isGitSha(s: string): boolean {
  return /^[0-9a-f]{7,40}$/i.test(s);
}

/** The revision a Change-Analysis `target` reads: a commit sha is that
 *  commit; `working` and synthetic keys are the working tree. */
export function targetRevision(target: string): Revision {
  return target !== WORKING && isGitSha(target) ? gitRevision(target) : WORKING;
}

/** Compact label: `working tree`, a 7-char sha, a branch, `snapshot N`. */
export function shortRevisionLabel(rev: Revision): string {
  if (rev === WORKING) return "working tree";
  const snap = snapshotIdOf(rev);
  if (snap !== null) return `snapshot ${snap}`;
  const value = rev.slice(rev.indexOf(":") + 1);
  return /^[0-9a-f]{13,40}$/i.test(value) ? value.slice(0, 7) : value;
}

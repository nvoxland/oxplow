/**
 * Versioned file access — the frontend mirror of Rust's `TreeVersion`.
 *
 * Every helper that reads, diffs, or compares file content takes one
 * of these as an explicit argument. There is no implicit
 * "the working tree" default: the duplication-scan bug
 * (where the working-tree was silently substituted for the analyzed
 * commit) is the kind of mistake this type exists to prevent.
 *
 * The runtime shape matches the generated `TreeVersion` in
 * `tauri-bridge/generated/bindings.ts` so this value can be passed
 * through to IPC without translation.
 */
export type FileVersion =
  | { kind: "disk" }
  | { kind: "ref"; ref: string }
  | { kind: "snapshot"; id: string };

/** Working-tree (on-disk) version. Use this only when you really mean
 *  "the file as it is right now," typically because the user is
 *  editing it. Read-only views over historical content should use a
 *  `ref` or `snapshot` version instead. */
export const DISK: FileVersion = { kind: "disk" };

/** Construct a ref-based version. `ref` is anything `git revparse`
 *  understands — sha, branch, tag, `HEAD`, `HEAD~3`, … */
export function refVersion(ref: string): FileVersion {
  return { kind: "ref", ref };
}

export function snapshotVersion(id: string): FileVersion {
  return { kind: "snapshot", id };
}

/** True when `s` looks like a git commit sha (7–40 hex chars) — used to
 *  tell a real ref-able Change-Analysis target apart from a synthetic
 *  diff-view tab-key like `endpoints:…` / `effort:…`. */
export function isGitSha(s: string): boolean {
  return /^[0-9a-f]{7,40}$/i.test(s);
}

/** The tree version a Change-Analysis `target` reads/scans against:
 *  `"working"` and synthetic diff-view keys → the working tree (disk);
 *  a commit sha → that ref. Centralized so the duplication scan, its
 *  freshness lookup, and the ref-stamping all agree. */
export function targetTreeVersion(target: string): FileVersion {
  if (target !== "working" && isGitSha(target)) return refVersion(target);
  return DISK;
}

/** Compact label for UI rendering: `disk`, `<sha7>`, `snap:<id7>`. */
export function shortLabelForVersion(version: FileVersion): string {
  switch (version.kind) {
    case "disk":
      return "disk";
    case "ref":
      return version.ref.length > 12 ? version.ref.slice(0, 7) : version.ref;
    case "snapshot":
      return `snap:${version.id.slice(0, 7)}`;
  }
}

/** Stable id-fragment, suitable for embedding in TabRef ids. */
export function versionIdFragment(version: FileVersion): string {
  switch (version.kind) {
    case "disk":
      return "disk";
    case "ref":
      return `ref:${version.ref}`;
    case "snapshot":
      return `snap:${version.id}`;
  }
}

/**
 * Coerce an unknown payload (typically read from `localStorage`) into
 * a valid `FileVersion`. Pre-versioning persisted refs had no
 * `version` field at all; treat those as `disk` (working tree) and
 * log a one-time warn so we can find any stale persistence by
 * looking at the UI logs.
 */
export function coerceFileVersion(raw: unknown, _context: string): FileVersion {
  if (raw && typeof raw === "object") {
    const r = raw as { kind?: unknown; ref?: unknown; id?: unknown };
    if (r.kind === "disk") return DISK;
    if (r.kind === "ref" && typeof r.ref === "string") {
      return { kind: "ref", ref: r.ref };
    }
    if (r.kind === "snapshot" && typeof r.id === "string") {
      return { kind: "snapshot", id: r.id };
    }
  }
  return DISK;
}

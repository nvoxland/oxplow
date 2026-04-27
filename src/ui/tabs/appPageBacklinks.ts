/**
 * App-page backlinks providers. App pages (Git Dashboard, Git History,
 * Uncommitted Changes, individual commits) don't fit the wiki-style
 * backlink model — there's no body to grep. Each declares a provider
 * here that maps `(payload, context)` → `BacklinkEntry[]` so the
 * shared `useBacklinks` hook can dispatch by `PageKind`.
 *
 * Pure: each provider's inputs are plain data slices that the host
 * fetches up front. Add a new app-page provider by:
 *   1. Adding a new branch to `AppBacklinkContext` for whatever data
 *      slice it needs.
 *   2. Implementing `(payload, ctx) => BacklinkEntry[]`.
 *   3. Registering it in `APP_PAGE_BACKLINKS`.
 *   4. Extending `useBacklinks` to fetch + memoize the new slice.
 */

import type { GitLogCommit } from "../api.js";
import type { BacklinkContext, BacklinkEntry } from "./backlinksIndex.js";
import {
  fileRef,
  gitCommitRef,
  gitDashboardRef,
  indexRef,
  noteRef,
  uncommittedChangesRef,
  workItemRef,
} from "./pageRefs.js";
import type { PageKind } from "./tabState.js";

export interface AppBacklinkContext extends BacklinkContext {
  /** Recent commits on the current branch (most recent first). Optional —
   *  may be empty when the host hasn't fetched yet. */
  recentLog?: GitLogCommit[];
  /** Files currently uncommitted (paths). */
  uncommittedPaths?: string[];
  /** Current branch name. */
  currentBranch?: string;
}

export type AppPageBacklinksProvider = (
  payload: unknown,
  ctx: AppBacklinkContext,
) => BacklinkEntry[];

export const APP_PAGE_BACKLINKS: Partial<Record<PageKind, AppPageBacklinksProvider>> = {
  "git-dashboard": gitDashboardBacklinks,
  "git-history": gitHistoryBacklinks,
  "uncommitted-changes": uncommittedChangesBacklinks,
  "git-commit": gitCommitBacklinks,
};

/** Pure helper: return the work items whose touched_files overlap any of `paths`. */
function workItemsTouching(paths: Iterable<string>, ctx: BacklinkContext): BacklinkEntry[] {
  const set = new Set(paths);
  const out: BacklinkEntry[] = [];
  for (const wi of ctx.workItems) {
    if (wi.touched_files.some((p) => set.has(p))) {
      out.push({ ref: workItemRef(wi.id), label: wi.title, subtitle: "work item" });
    }
  }
  return out;
}

/** Pure helper: return notes whose body mentions any path in `paths`. */
function notesMentioningPaths(paths: Iterable<string>, ctx: BacklinkContext): BacklinkEntry[] {
  const out: BacklinkEntry[] = [];
  for (const note of ctx.notes) {
    for (const path of paths) {
      if (note.body.includes(path)) {
        out.push({ ref: noteRef(note.slug), label: note.title || note.slug, subtitle: "note" });
        break;
      }
    }
  }
  return out;
}

/** Pure helper: notes mentioning a sha (full or 7-char prefix). */
function notesMentioningSha(sha: string, ctx: BacklinkContext): BacklinkEntry[] {
  if (!sha) return [];
  const short = sha.slice(0, 7);
  const out: BacklinkEntry[] = [];
  for (const note of ctx.notes) {
    if (note.body.includes(sha) || note.body.includes(short)) {
      out.push({ ref: noteRef(note.slug), label: note.title || note.slug, subtitle: "note" });
    }
  }
  return out;
}

export function gitDashboardBacklinks(_payload: unknown, ctx: AppBacklinkContext): BacklinkEntry[] {
  const out: BacklinkEntry[] = [
    { ref: indexRef("git-history"), label: "Git history", subtitle: "page" },
    { ref: uncommittedChangesRef(), label: "Uncommitted changes", subtitle: "page" },
  ];
  for (const c of (ctx.recentLog ?? []).slice(0, 5)) {
    out.push({
      ref: gitCommitRef(c.sha),
      label: `${c.sha.slice(0, 7)} · ${commitSubject(c)}`,
      subtitle: "commit",
    });
  }
  if (ctx.currentBranch) {
    for (const note of ctx.notes) {
      if (note.body.includes(ctx.currentBranch)) {
        out.push({ ref: noteRef(note.slug), label: note.title || note.slug, subtitle: "note" });
      }
    }
  }
  return dedupe(out);
}

export function gitHistoryBacklinks(_payload: unknown, ctx: AppBacklinkContext): BacklinkEntry[] {
  const out: BacklinkEntry[] = [
    { ref: gitDashboardRef(), label: "Git", subtitle: "page" },
    { ref: uncommittedChangesRef(), label: "Uncommitted changes", subtitle: "page" },
  ];
  // Notes mentioning any sha visible in the recent log.
  const seen = new Set<string>();
  for (const c of ctx.recentLog ?? []) {
    for (const entry of notesMentioningSha(c.sha, ctx)) {
      if (seen.has(entry.ref.id)) continue;
      seen.add(entry.ref.id);
      out.push(entry);
    }
  }
  return out;
}

export function uncommittedChangesBacklinks(_payload: unknown, ctx: AppBacklinkContext): BacklinkEntry[] {
  const paths = ctx.uncommittedPaths ?? [];
  const out: BacklinkEntry[] = paths.map((p) => ({
    ref: fileRef(p),
    label: p,
    subtitle: "file",
  }));
  out.push(...workItemsTouching(paths, ctx));
  out.push(...notesMentioningPaths(paths, ctx));
  return dedupe(out);
}

export function gitCommitBacklinks(payload: unknown, ctx: AppBacklinkContext): BacklinkEntry[] {
  const sha = (payload as { sha?: string } | null)?.sha ?? "";
  const out: BacklinkEntry[] = [
    { ref: indexRef("git-history"), label: "Git history", subtitle: "page" },
  ];
  // Files touched by this commit — derived from the recentLog if present.
  const c = (ctx.recentLog ?? []).find((entry) => entry.sha === sha);
  const paths: string[] = []; // recentLog doesn't carry per-commit file lists; the page-level
  //                              detail fetch does, but providers can't await. v1 keeps the
  //                              file/work-item/note links empty here unless ctx is enriched.
  out.push(...workItemsTouching(paths, ctx));
  out.push(...notesMentioningSha(sha, ctx));
  if (c) {
    // Cross-link to the previous and next commit in the visible log.
    const idx = (ctx.recentLog ?? []).indexOf(c);
    const prev = (ctx.recentLog ?? [])[idx + 1];
    const next = (ctx.recentLog ?? [])[idx - 1];
    if (prev) out.push({ ref: gitCommitRef(prev.sha), label: `${prev.sha.slice(0, 7)} · ${commitSubject(prev)}`, subtitle: "previous commit" });
    if (next) out.push({ ref: gitCommitRef(next.sha), label: `${next.sha.slice(0, 7)} · ${commitSubject(next)}`, subtitle: "next commit" });
  }
  return dedupe(out);
}

function commitSubject(c: GitLogCommit): string {
  return c.commit.message.split("\n", 1)[0] ?? "";
}

function dedupe(entries: BacklinkEntry[]): BacklinkEntry[] {
  const seen = new Set<string>();
  const out: BacklinkEntry[] = [];
  for (const e of entries) {
    if (seen.has(e.ref.id)) continue;
    seen.add(e.ref.id);
    out.push(e);
  }
  return out;
}

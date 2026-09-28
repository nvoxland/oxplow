/**
 * Helpers for constructing `TabRef` values consistently. Centralizing the
 * id format keeps cross-component links and ⌘K open-by-id stable.
 */

import type { TabRef } from "./tabState.js";
import {
  DISK,
  type FileVersion,
  revForVersion,
  versionFromIdFragment,
  versionFromRev,
  versionIdFragment,
} from "../file-version.js";
import type { DiffEndpoint, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { formatRef, parseRef } from "../refs/ref.js";

/** Tab ids for the entity kinds are canonical refs (`.context/refs.md`):
 *  `file:<path>[@rev]`, `dir:<path>`, `work_item:oxplow:tsk42`,
 *  `commit:<sha>`, `wiki:<slug>`, `metric:<key>`. Everything else here is
 *  a shell route whose id is still hand-formatted (they move to
 *  `page:<name>` in P1.3b). */
function canonicalId(kind: string, id: string, rev: string | null = null): string {
  return formatRef({ kind, id, rev, frag: null });
}

/** Tasks are work items under the oxplow provider (`oxplow:tsk42`). */
const OXPLOW_PROVIDER = "oxplow:";

export function agentRef(): TabRef {
  return { id: "agent", kind: "agent", payload: null };
}

/**
 * Construct a file-tab ref. `version` is required: callers MUST
 * declare which version of the tree they want to view, even if the
 * answer is `DISK` (the working tree). This rule exists because the
 * "implicit working tree" assumption is what made the duplication
 * scan show stale, mismatched line ranges in commit-target analysis.
 *
 * The id is the canonical ref: a working-tree file is `file:<path>`; a
 * historical view carries its revision (`file:<path>@git:HEAD`,
 * `@snap:<id>`) so it is a distinct tab from the working-tree view.
 */
export function fileRef(path: string, version: FileVersion = DISK): TabRef {
  return { id: canonicalId("file", path, revForVersion(version)), kind: "file", payload: { path, version } };
}

export function directoryRef(path: string): TabRef {
  // Trailing slash is normalized away — `[[src/]]` and `[[src]]` (when
  // ever the parser admits the latter) collapse to one tab.
  const bare = path.replace(/\/+$/, "");
  return { id: canonicalId("dir", bare), kind: "dir", payload: { path: bare } };
}

export interface DiffPayload {
  path: string;
  fromRef?: string | null;
  toRef?: string | null;
  /** Free-form short label, e.g. "wi-142", "snapshot 4h ago". */
  labelOverride?: string | null;
}

export function diffRef(payload: DiffPayload): TabRef {
  const key = [payload.path, payload.fromRef ?? "", payload.toRef ?? "", payload.labelOverride ?? ""].join("|");
  return { id: `diff:${key}`, kind: "diff", payload };
}

export interface DuplicateBlockPayload {
  leftPath: string;
  leftStart: number;
  leftEnd: number;
  /** Tree version the LEFT side was scanned against. The page reads
   *  file content at this version so highlighted line ranges match
   *  the displayed text — never silently substitutes the working
   *  tree. */
  leftVersion: FileVersion;
  rightPath: string;
  rightStart: number;
  rightEnd: number;
  rightVersion: FileVersion;
}

/**
 * Side-by-side view of a duplicate-block finding. Both ranges are
 * loaded at the version the scan ran against and highlighted; the
 * editors are scrolled so the two start lines line up at the top of
 * the viewport.
 */
export function duplicateBlockRef(payload: DuplicateBlockPayload): TabRef {
  const lv = versionIdFragment(payload.leftVersion);
  const rv = versionIdFragment(payload.rightVersion);
  const id = `dup:${payload.leftPath}:${payload.leftStart}-${payload.leftEnd}@${lv}::${payload.rightPath}:${payload.rightStart}-${payload.rightEnd}@${rv}`;
  return { id, kind: "duplicate-block", payload };
}

export function wikiPageRef(slug: string): TabRef {
  return { id: canonicalId("wiki", slug), kind: "wiki", payload: { slug } };
}

export function wikiFreshnessRef(slug: string): TabRef {
  return { id: `wiki-freshness:${slug}`, kind: "wiki-freshness", payload: { slug } };
}

/** A task page. `itemId` is the `tsk<n>` id; the tab id is the canonical
 *  work-item ref under the oxplow provider (`work_item:oxplow:tsk42`). */
export function taskRef(itemId: string): TabRef {
  return { id: canonicalId("work_item", `${OXPLOW_PROVIDER}${itemId}`), kind: "work_item", payload: { itemId } };
}

/** Open one metric's detail page, optionally scoped to an effort's window —
 *  the task-page metrics-panel drill-in ("In this effort" before→after +
 *  further exploration). Its own page kind (`metric`); Metrics and the
 *  dashboard tiles both navigate into it. */
export function metricRef(metricKey: string): TabRef {
  return { id: canonicalId("metric", metricKey), kind: "metric", payload: { metricKey } };
}

export function indexRef(kind: "tasks" | "done-work" | "backlog" | "archived" | "wiki-index" | "files" | "comments" | "local-history" | "local-history-full" | "local-history-by-commit-full" | "git-history" | "hook-events" | "terminal" | "settings" | "metrics-recorded" | "dashboards" | "explore-data"): TabRef {
  return { id: kind, kind, payload: null };
}

/** One user-created dashboard — a payload-bearing page kind (like
 *  `metric`). The dashboard id (`dsh<n>`) rides in both the tab id and
 *  the payload so a history-restored tab (no payload) still resolves via
 *  `refFromTabId`. */
export function customDashboardRef(id: string): TabRef {
  return { id: `custom-dashboard:${id}`, kind: "custom-dashboard", payload: { id } };
}

/** The Dashboards index — the list of the user's custom dashboards. */
export function dashboardsRef(): TabRef {
  return indexRef("dashboards");
}

/** The Metrics page — every catalogued definition with latest value, trend
 *  sparkline, capture branch, sample count.
 *
 *  `metricsIndexRef`, not `metricsRef`, to stay clearly distinct from
 *  {@link metricRef} (one metric's detail page). The `metrics-recorded` kind id
 *  it returns keeps its old spelling on purpose — it is baked into persisted tab
 *  ids and section-collapse keys, so renaming it would drop saved tabs (tsk222). */
export function metricsIndexRef(): TabRef {
  return indexRef("metrics-recorded");
}

/** The global Comments inbox: every comment in the current stream. */
export function commentsRef(): TabRef {
  return indexRef("comments");
}

/** The Terminal page: a plain interactive shell rooted at the worktree dir. */
export function terminalRef(): TabRef {
  return indexRef("terminal");
}

/** Convenience helper for the new HookEventsPage. */
export function hookEventsRef(): TabRef {
  return indexRef("hook-events");
}

/**
 * Named ref helpers for the four work pages that replaced the legacy
 * single AllWorkPage. Mirrors the GitDashboard pattern
 * (`gitDashboardRef`, `uncommittedChangesRef`) so call sites read as
 * intent rather than as stringly-typed `indexRef("…")`.
 */
export function tasksRef(): TabRef {
  return indexRef("tasks");
}
/** @deprecated Use `tasksRef()` instead. Kept as an alias for one
 *  release so existing call sites and persisted refs keep working. */
export function planWorkRef(): TabRef {
  return tasksRef();
}
export function doneWorkRef(): TabRef {
  return indexRef("done-work");
}
export function backlogRef(): TabRef {
  return indexRef("backlog");
}
export function archivedRef(): TabRef {
  return indexRef("archived");
}

/** Git Dashboard — committed-history rollup page. */
export function gitDashboardRef(): TabRef {
  return { id: "git-dashboard", kind: "git-dashboard", payload: null };
}

/** Diff view of a single captured snapshot — framed as `[prev → N]`
 *  (the previous capture in the stream is the start; the page resolves
 *  it on load). Drill-in from the Local History dashboard, file version
 *  history, and snapshot backlinks. */
export function snapshotRef(snapshotId: number): TabRef {
  return {
    id: `diff-view:snapshot:${snapshotId}`,
    kind: "diff-view",
    payload: { mode: "snapshot", snapshotId },
  };
}

/**
 * The diff view (`diff-view` kind) reframes the old snapshot detail
 * page as an explicit start→end diff. Two entry shapes, both rendered
 * by `DiffViewPage`:
 *
 * - **effort** (`diff-view:effort:<effortId>`) — resolves the effort's
 *   own start/end snapshot bracket on load (survives a cold history
 *   reopen where only the id is in the tab id), carrying the task title
 *   + "in progress" state.
 * - **endpoints** (`diff-view:endpoints:<start>..<end>`) — an explicit
 *   pair of snapshot/commit/working endpoints. `start = null` diffs
 *   `end` against the empty tree (everything added).
 * - **snapshot** (`diff-view:snapshot:<N>`) — a single capture, framed
 *   as `[prev → N]`; the page resolves the previous snapshot on load.
 */
export type DiffViewPayload =
  | { mode: "snapshot"; snapshotId: number }
  | { mode: "effort"; effortId: string }
  | { mode: "endpoints"; start: DiffEndpoint | null; end: DiffEndpoint };

/** Stable single-token encoding of one endpoint for the tab id. */
function encodeEndpoint(ep: DiffEndpoint | null): string {
  if (ep === null) return "none";
  switch (ep.kind) {
    case "snapshot":
      return `s${ep.snapshot_id}`;
    case "commit":
      return `c${ep.sha}`;
    case "working":
      return "w";
  }
}

/** Inverse of `encodeEndpoint` — used by `refFromTabId` to rebuild an
 *  endpoint diff from its tab id alone (history reopen). */
function decodeEndpoint(token: string): DiffEndpoint | null {
  if (token === "none") return null;
  if (token === "w") return { kind: "working" };
  if (token.startsWith("s")) return { kind: "snapshot", snapshot_id: Number(token.slice(1)) };
  if (token.startsWith("c")) return { kind: "commit", sha: token.slice(1) };
  return null;
}

/** Diff view scoped to one effort — resolves the effort's start/end
 *  snapshot bracket on load. The 'View diff' button on a completed
 *  effort points here. */
export function effortDiffRef(effortId: string): TabRef {
  return {
    id: `diff-view:effort:${effortId}`,
    kind: "diff-view",
    payload: { mode: "effort", effortId },
  };
}

/** Diff view between two explicit endpoints (snapshot / commit /
 *  working). `start = null` diffs `end` against the empty tree. */
export function endpointDiffRef(start: DiffEndpoint | null, end: DiffEndpoint): TabRef {
  return {
    id: `diff-view:endpoints:${encodeEndpoint(start)}..${encodeEndpoint(end)}`,
    kind: "diff-view",
    payload: { mode: "endpoints", start, end },
  };
}

/** Uncommitted Changes — the working tree's changed files, commit form
 *  and the `uncommitted` lens slot. */
export function uncommittedChangesRef(): TabRef {
  return { id: "uncommitted-changes", kind: "uncommitted-changes", payload: null };
}

/** Single git commit page. */
export function gitCommitRef(sha: string): TabRef {
  return { id: canonicalId("commit", sha), kind: "commit", payload: { sha } };
}

export type DashboardKind = "visits";

export function dashboardRef(variant: DashboardKind): TabRef {
  return { id: `dashboard:${variant}`, kind: "dashboard", payload: { variant } };
}

/**
 * Form pages introduced by phase 5e. These replace the legacy modal
 * dialogs (NewStreamModal / NewtasksModal / Stream-Thread settings)
 * with a focused full-tab workspace, matching `SettingsPage`.
 */

export interface NewtasksPayload {
  /** Optional pre-selected parent epic id. */
  parentId?: string | null;
  /** Optional default priority. */
  initialPriority?: string | null;
}

export function newStreamRef(): TabRef {
  return { id: "new-stream", kind: "new-stream", payload: null };
}

export function newTaskRef(payload: NewtasksPayload = {}): TabRef {
  // Use a stable id so re-opening the page reuses the existing tab
  // rather than stacking duplicates. "Save and Another" relies on the
  // form re-mounting in place; the page reads its initial values on
  // mount, so callers wanting different defaults should `closeTab`
  // before opening with new payload.
  return { id: "new-task", kind: "new-task", payload };
}

export function streamSettingsRef(streamId: string): TabRef {
  return { id: `stream-settings:${streamId}`, kind: "stream-settings", payload: { streamId } };
}

export function threadSettingsRef(threadId: string): TabRef {
  return { id: `thread-settings:${threadId}`, kind: "thread-settings", payload: { threadId } };
}

/** A lens page (`lens:<extension>/<slug>`): a user/agent-built query
 *  over the semantic layer from `oxplow/extensions/`. */
/** A lens page. `params` (e.g. a slot's `{ effort_id }`) ride in the id
 *  as a sorted query string, so the tab, its history and bookmarks reopen
 *  the lens with the same values. */
export function lensRef(lensId: string, params?: Record<string, SqlCell>): TabRef {
  const keys = Object.keys(params ?? {}).sort();
  if (!params || keys.length === 0) return { id: `lens:${lensId}`, kind: "lens", payload: { lensId } };
  const qs = new URLSearchParams(keys.map((k) => [k, params[k] === null ? "" : String(params[k])])).toString();
  return { id: `lens:${lensId}?${qs}`, kind: "lens", payload: { lensId, params } };
}

/** Parse a lens tab id's `<lensId>?k=v` tail back into a ref. Numeric
 *  values come back as numbers, as the params form reads them. */
function lensRefFromTail(tail: string): TabRef {
  const q = tail.indexOf("?");
  if (q === -1) return lensRef(tail);
  const params: Record<string, SqlCell> = {};
  for (const [k, v] of new URLSearchParams(tail.slice(q + 1))) {
    params[k] = v === "" ? null : /^-?\d+(\.\d+)?$/.test(v) ? Number(v) : v;
  }
  return lensRef(tail.slice(0, q), params);
}

export function closedThreadsRef(): TabRef {
  return { id: "closed-threads", kind: "closed-threads", payload: null };
}

/** Async-op error detail page. Id is scoped to the error id so each
 *  failure gets its own tab; closing it discards the view, not the
 *  store entry. */
export function opErrorRef(errorId: string): TabRef {
  return { id: `op-error:${errorId}`, kind: "op-error", payload: { errorId } };
}

export interface ExternalUrlPayload {
  url: string;
}

/**
 * Tab ref for an external (http/https) URL rendered inside a sandboxed
 * <webview> in the app. The URL is used as the tab id so reopening the
 * same link reuses the existing tab rather than stacking duplicates.
 *
 * Callers MUST validate the URL through `classifyExternalUrl` from
 * `src/ui/external-url-allowlist.ts` before constructing this ref —
 * the renderer trusts that the payload has already been gated.
 */
export function externalUrlRef(url: string): TabRef {
  return { id: `external-url:${url}`, kind: "external-url", payload: { url } };
}

/**
 * Reconstruct a full `TabRef` (with payload) from a tab id alone.
 *
 * Page-visit history rows persist only the id (`page_id`) and kind, not
 * the ref payload — so a naive `{ id, kind, payload: null }` rebuild
 * leaves payload-bearing pages broken (a `file` ref with no `path`
 * never opens; `wiki`/`work_item`/etc. render empty). Entity kinds parse
 * through the canonical grammar (so `:` inside `work_item:oxplow:tsk42`
 * is fine); shell routes parse their own hand-formatted tails.
 * Index/dashboard kinds carry no payload, so the id IS the kind and the
 * fallback is fine.
 */
export function refFromTabId(id: string): TabRef {
  const canonical = parseRef(id);
  if (canonical) {
    switch (canonical.kind) {
      case "file":
        return fileRef(canonical.id, versionFromRev(canonical.rev));
      case "dir":
        return directoryRef(canonical.id);
      case "wiki":
        return wikiPageRef(canonical.id);
      case "work_item":
        return taskRef(canonical.id.startsWith(OXPLOW_PROVIDER) ? canonical.id.slice(OXPLOW_PROVIDER.length) : canonical.id);
      case "commit":
        return gitCommitRef(canonical.id);
      case "metric":
        return metricRef(canonical.id);
      default:
        break;
    }
  }
  return routeFromTabId(id);
}

/** The shell routes: ids whose tail is not a canonical ref id. */
function routeFromTabId(id: string): TabRef {
  const colon = id.indexOf(":");
  const scheme = colon === -1 ? id : id.slice(0, colon);
  const rest = colon === -1 ? "" : id.slice(colon + 1);
  switch (scheme) {
    case "lens":
      return lensRefFromTail(rest);
    case "wiki-freshness":
      return wikiFreshnessRef(rest);
    case "custom-dashboard":
      // `rest` is the `dsh<n>` id.
      return customDashboardRef(rest);
    case "dashboard": {
      const variants: readonly string[] = ["visits"];
      return variants.includes(rest)
        ? dashboardRef(rest as DashboardKind)
        : { id, kind: "dashboard", payload: null };
    }
    case "diff-view": {
      // `rest` is `snapshot:<N>` | `effort:<id>` | `endpoints:<start>..<end>`.
      const sub = rest.indexOf(":");
      const subScheme = sub === -1 ? rest : rest.slice(0, sub);
      const subRest = sub === -1 ? "" : rest.slice(sub + 1);
      if (subScheme === "snapshot") {
        const n = Number(subRest);
        if (Number.isFinite(n)) return snapshotRef(n);
      }
      if (subScheme === "effort") return effortDiffRef(subRest);
      if (subScheme === "endpoints") {
        const [startTok, endTok] = subRest.split("..");
        const end = decodeEndpoint(endTok ?? "");
        if (end) return endpointDiffRef(decodeEndpoint(startTok ?? "none"), end);
      }
      return { id, kind: "diff-view", payload: null };
    }
    case "dup": {
      const dup = duplicateBlockFromTail(rest);
      return dup ?? { id, kind: "duplicate-block", payload: null };
    }
    case "uncommitted-changes":
      return uncommittedChangesRef();
    case "external-url":
      return externalUrlRef(rest);
    case "op-error":
      return opErrorRef(rest);
    case "stream-settings":
      return streamSettingsRef(rest);
    case "thread-settings":
      return threadSettingsRef(rest);
    default:
      // Index/dashboard kinds (`tasks`, `files`, `git-dashboard`, …) and
      // any unknown scheme: no payload needed; the id is the kind.
      return { id, kind: scheme as TabRef["kind"], payload: null };
  }
}

/** Parse the tail `duplicateBlockRef` writes:
 *  `<left>:<a>-<b>@<ver>::<right>:<c>-<d>@<ver>`. */
function duplicateBlockFromTail(tail: string): TabRef | null {
  const sides = tail.split("::");
  if (sides.length !== 2) return null;
  const side = (s: string) => {
    const m = /^(.+):(\d+)-(\d+)@(.+)$/.exec(s);
    if (!m) return null;
    const version = versionFromIdFragment(m[4]!);
    return version ? { path: m[1]!, start: Number(m[2]), end: Number(m[3]), version } : null;
  };
  const l = side(sides[0]!);
  const r = side(sides[1]!);
  if (!l || !r) return null;
  return duplicateBlockRef({
    leftPath: l.path, leftStart: l.start, leftEnd: l.end, leftVersion: l.version,
    rightPath: r.path, rightStart: r.start, rightEnd: r.end, rightVersion: r.version,
  });
}

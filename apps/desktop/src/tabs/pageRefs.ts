/**
 * Helpers for constructing `TabRef` values consistently. Every tab id is
 * a canonical ref (`.context/refs.md`), built only here — never
 * hand-format an id.
 *
 * - An **entity** page's id is the entity's own ref: `file:<path>[@rev]`,
 *   `dir:<path>`, `wiki:<slug>`, `work_item:oxplow:tsk42`, `commit:<sha>`,
 *   `metric:<key>`, `lens:<ext>/<slug>[?params]`.
 * - A **route** (a page of the shell, not a thing in the graph) is
 *   `page:<name>[?params]` — `page:tasks`, `page:diff-view?effort=eff9`.
 *   Routes never appear in `page_ref`.
 *
 * `refFromTabId` is the inverse of every constructor here: history,
 * bookmarks and the Go To page persist only the id, so each payload
 * must be rebuildable from it (the round-trip test in `pageRefs.test.ts`
 * covers every constructor).
 */

import type { PageKind, RoutePageKind, TabRef } from "./tabState.js";
import {
  DISK,
  type FileVersion,
  revForVersion,
  versionFromIdFragment,
  versionFromRev,
  versionIdFragment,
} from "../file-version.js";
import type { DiffEndpoint, SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { escapeId, formatRef, parseRef } from "../refs/ref.js";

function canonicalId(kind: string, id: string, rev: string | null = null): string {
  return formatRef({ kind, id, rev, frag: null });
}

/** Tasks are work items under the oxplow provider (`oxplow:tsk42`). */
const OXPLOW_PROVIDER = "oxplow:";

// ---------------------------------------------------------------------------
// Query params: `?k=v&k=v` after a route name or a lens id.
//
// Values are percent-encoded the way `URLSearchParams` reads them, but only
// the characters that would break the id are escaped — the query syntax
// (`&`, `=`, `+`, `%`) and the ref grammar's reserved `@` and `#` — so
// `path=src/a.ts&left=ref:abc` stays readable. The encoded text is a
// valid canonical ref of its kind (`page:` / `lens:`) as written, so it
// is parsed from the RAW text after the kind, not from `parseRef`'s
// decoded id (which would turn an escaped `&` back into a separator).
// ---------------------------------------------------------------------------

type ParamValue = string | number | null | undefined;

function encodeParamValue(v: string): string {
  return encodeURIComponent(v)
    .replace(/%2F/gi, "/")
    .replace(/%3A/gi, ":")
    .replace(/%3F/gi, "?")
    .replace(/%2C/gi, ",")
    .replace(/%20/g, "+");
}

/** `k=v&k=v` in the given key order; `null`/`undefined` values are skipped. */
function encodeParams(params: Record<string, ParamValue>): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v === null || v === undefined) continue;
    parts.push(`${encodeParamValue(k)}=${encodeParamValue(String(v))}`);
  }
  return parts.join("&");
}

/** Split `<head>?<qs>` at the first `?`; `head` is percent-decoded. */
function splitParams(raw: string): { head: string; params: URLSearchParams } {
  const q = raw.indexOf("?");
  const head = q === -1 ? raw : raw.slice(0, q);
  return { head: decodeURIComponent(head), params: new URLSearchParams(q === -1 ? "" : raw.slice(q + 1)) };
}

/** The id of a shell route: `page:<name>[?params]`. */
function pageId(name: RoutePageKind, params?: Record<string, ParamValue>): string {
  const qs = params ? encodeParams(params) : "";
  return qs ? `page:${name}?${qs}` : `page:${name}`;
}

function route(name: RoutePageKind, payload: unknown = null, params?: Record<string, ParamValue>): TabRef {
  return { id: pageId(name, params), kind: name, payload };
}

// ---------------------------------------------------------------------------
// Entity pages
// ---------------------------------------------------------------------------

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

/** The working-tree file a tab id shows, or `null` when the id is not a
 *  file or pins a revision (a read-only viewer, not the editor's file). */
export function diskFilePath(tabId: string): string | null {
  const r = parseRef(tabId);
  return r && r.kind === "file" && r.rev === null ? r.id : null;
}

export function directoryRef(path: string): TabRef {
  // Trailing slash is normalized away — `[[src/]]` and `[[src]]` (when
  // ever the parser admits the latter) collapse to one tab.
  const bare = path.replace(/\/+$/, "");
  return { id: canonicalId("dir", bare), kind: "dir", payload: { path: bare } };
}

export function wikiPageRef(slug: string): TabRef {
  return { id: canonicalId("wiki", slug), kind: "wiki", payload: { slug } };
}

/** A task page. `itemId` is the `tsk<n>` id; the tab id is the canonical
 *  work-item ref under the oxplow provider (`work_item:oxplow:tsk42`). */
export function taskRef(itemId: string): TabRef {
  return { id: canonicalId("work_item", `${OXPLOW_PROVIDER}${itemId}`), kind: "work_item", payload: { itemId } };
}

/** Single git commit page. */
export function gitCommitRef(sha: string): TabRef {
  return { id: canonicalId("commit", sha), kind: "commit", payload: { sha } };
}

/** Open one metric's detail page, optionally scoped to an effort's window —
 *  the task-page metrics-panel drill-in ("In this effort" before→after +
 *  further exploration). Its own page kind (`metric`); Metrics and the
 *  dashboard tiles both navigate into it. */
export function metricRef(metricKey: string): TabRef {
  return { id: canonicalId("metric", metricKey), kind: "metric", payload: { metricKey } };
}

/** A lens page (`lens:<extension>/<slug>`): a user/agent-built query
 *  over the semantic layer from `oxplow/extensions/`. `params` (e.g. a
 *  slot's `{ effort_id }`) ride in the id as a sorted query string, so
 *  the tab, its history and bookmarks reopen the lens with the same
 *  values. */
export function lensRef(lensId: string, params?: Record<string, SqlCell>): TabRef {
  const keys = Object.keys(params ?? {}).sort();
  const head = `lens:${escapeId(lensId)}`;
  if (!params || keys.length === 0) return { id: head, kind: "lens", payload: { lensId } };
  const qs = encodeParams(Object.fromEntries(keys.map((k) => [k, params[k] === null ? "" : String(params[k])])));
  return { id: `${head}?${qs}`, kind: "lens", payload: { lensId, params } };
}

/** Parse a lens id's raw `<lensId>?k=v` tail back into a ref. Numeric
 *  values come back as numbers, as the params form reads them. */
function lensRefFromTail(tail: string): TabRef {
  const { head, params } = splitParams(tail);
  const out: Record<string, SqlCell> = {};
  for (const [k, v] of params) {
    out[k] = v === "" ? null : /^-?\d+(\.\d+)?$/.test(v) ? Number(v) : v;
  }
  return lensRef(head, out);
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

export function agentRef(): TabRef {
  return route("agent");
}

/** The agent tab's id — the one tab every thread always has. */
export const AGENT_TAB_ID: string = agentRef().id;

export type IndexKind =
  | "tasks"
  | "done-work"
  | "backlog"
  | "archived"
  | "wiki-index"
  | "files"
  | "comments"
  | "local-history"
  | "local-history-full"
  | "local-history-by-commit-full"
  | "git-history"
  | "hook-events"
  | "terminal"
  | "settings"
  | "metrics-recorded"
  | "dashboards"
  | "explore-data";

export function indexRef(kind: IndexKind): TabRef {
  return route(kind);
}

export interface DiffPayload {
  path: string;
  leftVersion: FileVersion;
  rightVersion: FileVersion;
  labelOverride: string | null;
}

/** Stable id for a diff tab. Keyed off the path + both side versions
 *  + label override so re-opening the same diff with a new revealLine
 *  reuses the existing tab. */
export function computeDiffId(spec: DiffSpec): string {
  return pageId("diff", {
    path: spec.path,
    left: versionIdFragment(spec.leftVersion),
    right: versionIdFragment(spec.rightVersion),
    label: spec.labelOverride ?? null,
  });
}

/** The TabRef for a diff page. The payload is what `handleOpenDiff`
 *  registers; the full `DiffSpec` lives in the diff spec registry. */
export function diffRef(spec: DiffSpec): TabRef {
  const payload: DiffPayload = {
    path: spec.path,
    leftVersion: spec.leftVersion,
    rightVersion: spec.rightVersion,
    labelOverride: spec.labelOverride ?? null,
  };
  return { id: computeDiffId(spec), kind: "diff", payload };
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
  return route("duplicate-block", payload, {
    left: payload.leftPath,
    left_lines: `${payload.leftStart}-${payload.leftEnd}`,
    left_at: versionIdFragment(payload.leftVersion),
    right: payload.rightPath,
    right_lines: `${payload.rightStart}-${payload.rightEnd}`,
    right_at: versionIdFragment(payload.rightVersion),
  });
}

export function wikiFreshnessRef(slug: string): TabRef {
  return route("wiki-freshness", { slug }, { slug });
}

/** One user-created dashboard. The dashboard id (`dsh<n>`) rides in
 *  both the id and the payload so a history-restored tab resolves via
 *  `refFromTabId`. */
export function customDashboardRef(id: string): TabRef {
  return route("custom-dashboard", { id }, { id });
}

/** The Dashboards index — the list of the user's custom dashboards. */
export function dashboardsRef(): TabRef {
  return indexRef("dashboards");
}

/** The Metrics page — every catalogued definition with latest value, trend
 *  sparkline, capture branch, sample count.
 *
 *  `metricsIndexRef`, not `metricsRef`, to stay clearly distinct from
 *  {@link metricRef} (one metric's detail page). */
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

export function hookEventsRef(): TabRef {
  return indexRef("hook-events");
}

/**
 * Named ref helpers for the four work pages. Mirrors the GitDashboard
 * pattern (`gitDashboardRef`, `uncommittedChangesRef`) so call sites
 * read as intent rather than as stringly-typed `indexRef("…")`.
 */
export function tasksRef(): TabRef {
  return indexRef("tasks");
}
/** @deprecated Use `tasksRef()` instead. */
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
  return route("git-dashboard");
}

/**
 * The diff view (`diff-view` route) frames a change as an explicit
 * start→end diff. Three entry shapes, all rendered by `DiffViewPage`:
 *
 * - **snapshot** (`?snapshot=<N>`) — a single capture, framed as
 *   `[prev → N]`; the page resolves the previous snapshot on load.
 * - **effort** (`?effort=<effortId>`) — resolves the effort's own
 *   start/end snapshot bracket on load, carrying the task title + "in
 *   progress" state.
 * - **endpoints** (`?start=<tok>&end=<tok>`) — an explicit pair of
 *   snapshot/commit/working endpoints. `start = null` (`none`) diffs
 *   `end` against the empty tree (everything added).
 */
export type DiffViewPayload =
  | { mode: "snapshot"; snapshotId: number }
  | { mode: "effort"; effortId: string }
  | { mode: "endpoints"; start: DiffEndpoint | null; end: DiffEndpoint };

/** Diff view of a single captured snapshot. Drill-in from the Local
 *  History dashboard, file version history, and snapshot backlinks. */
export function snapshotRef(snapshotId: number): TabRef {
  return route("diff-view", { mode: "snapshot", snapshotId }, { snapshot: snapshotId });
}

/** Stable single-token encoding of one endpoint for the id. */
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
  return route("diff-view", { mode: "effort", effortId }, { effort: effortId });
}

/** Diff view between two explicit endpoints (snapshot / commit /
 *  working). `start = null` diffs `end` against the empty tree. */
export function endpointDiffRef(start: DiffEndpoint | null, end: DiffEndpoint): TabRef {
  return route("diff-view", { mode: "endpoints", start, end }, { start: encodeEndpoint(start), end: encodeEndpoint(end) });
}

/** Uncommitted Changes — the working tree's changed files, commit form
 *  and the `uncommitted` lens slot. */
export function uncommittedChangesRef(): TabRef {
  return route("uncommitted-changes");
}

export type DashboardKind = "visits";

export function dashboardRef(variant: DashboardKind): TabRef {
  return route("dashboard", { variant }, { variant });
}

/**
 * Form pages. These replaced the legacy modal dialogs (NewStreamModal /
 * NewtasksModal / Stream-Thread settings) with a focused full-tab
 * workspace, matching `SettingsPage`.
 */

export interface NewtasksPayload {
  /** Optional pre-selected parent epic id. */
  parentId?: string | null;
  /** Optional default priority. */
  initialPriority?: string | null;
}

export function newStreamRef(): TabRef {
  return route("new-stream");
}

/** The id is stable (no params) so re-opening the page reuses the
 *  existing tab rather than stacking duplicates; the defaults are read
 *  on mount, so a history reopen starts from an empty form. Callers
 *  wanting different defaults should `closeTab` before opening with a
 *  new payload. */
export function newTaskRef(payload: NewtasksPayload = {}): TabRef {
  return route("new-task", payload);
}

export function streamSettingsRef(streamId: string): TabRef {
  return route("stream-settings", { streamId }, { stream: streamId });
}

export function threadSettingsRef(threadId: string): TabRef {
  return route("thread-settings", { threadId }, { thread: threadId });
}

export function closedThreadsRef(): TabRef {
  return route("closed-threads");
}

/** Async-op error detail page. Scoped to the error id so each failure
 *  gets its own tab; closing it discards the view, not the store entry. */
export function opErrorRef(errorId: string): TabRef {
  return route("op-error", { errorId }, { id: errorId });
}

export interface ExternalUrlPayload {
  url: string;
}

/**
 * Tab ref for an external (http/https) URL rendered inside a sandboxed
 * <webview> in the app. The URL keys the id so reopening the same link
 * reuses the existing tab rather than stacking duplicates.
 *
 * Callers MUST validate the URL through `classifyExternalUrl` from
 * `src/ui/external-url-allowlist.ts` before constructing this ref —
 * the renderer trusts that the payload has already been gated.
 */
export function externalUrlRef(url: string): TabRef {
  return route("external-url", { url }, { url });
}

// ---------------------------------------------------------------------------
// Inverse: id → ref
// ---------------------------------------------------------------------------

/** How each route rebuilds itself from its params. Exhaustive over
 *  `RoutePageKind`, so a new route can't ship without its inverse. */
const ROUTES: Record<RoutePageKind, (params: URLSearchParams) => TabRef | null> = {
  agent: () => agentRef(),
  tasks: () => indexRef("tasks"),
  "done-work": () => indexRef("done-work"),
  backlog: () => indexRef("backlog"),
  archived: () => indexRef("archived"),
  "wiki-index": () => indexRef("wiki-index"),
  files: () => indexRef("files"),
  comments: () => indexRef("comments"),
  "local-history": () => indexRef("local-history"),
  "local-history-full": () => indexRef("local-history-full"),
  "local-history-by-commit-full": () => indexRef("local-history-by-commit-full"),
  "git-history": () => indexRef("git-history"),
  "hook-events": () => indexRef("hook-events"),
  terminal: () => indexRef("terminal"),
  settings: () => indexRef("settings"),
  "metrics-recorded": () => indexRef("metrics-recorded"),
  dashboards: () => indexRef("dashboards"),
  "explore-data": () => indexRef("explore-data"),
  "git-dashboard": () => gitDashboardRef(),
  "uncommitted-changes": () => uncommittedChangesRef(),
  "new-stream": () => newStreamRef(),
  "new-task": () => newTaskRef(),
  "closed-threads": () => closedThreadsRef(),
  diff: (p) => {
    const path = p.get("path");
    const left = versionFromIdFragment(p.get("left") ?? "");
    const right = versionFromIdFragment(p.get("right") ?? "");
    if (!path || !left || !right) return null;
    return diffRef({ path, leftVersion: left, rightVersion: right, baseLabel: "", labelOverride: p.get("label") ?? undefined });
  },
  "diff-view": (p) => {
    const snapshot = p.get("snapshot");
    if (snapshot !== null) {
      const n = Number(snapshot);
      return Number.isFinite(n) ? snapshotRef(n) : null;
    }
    const effort = p.get("effort");
    if (effort) return effortDiffRef(effort);
    const end = decodeEndpoint(p.get("end") ?? "");
    if (end) return endpointDiffRef(decodeEndpoint(p.get("start") ?? "none"), end);
    return null;
  },
  "duplicate-block": (p) => {
    const side = (path: string | null, lines: string | null, at: string | null) => {
      const m = /^(\d+)-(\d+)$/.exec(lines ?? "");
      const version = versionFromIdFragment(at ?? "");
      return path && m && version ? { path, start: Number(m[1]), end: Number(m[2]), version } : null;
    };
    const l = side(p.get("left"), p.get("left_lines"), p.get("left_at"));
    const r = side(p.get("right"), p.get("right_lines"), p.get("right_at"));
    if (!l || !r) return null;
    return duplicateBlockRef({
      leftPath: l.path, leftStart: l.start, leftEnd: l.end, leftVersion: l.version,
      rightPath: r.path, rightStart: r.start, rightEnd: r.end, rightVersion: r.version,
    });
  },
  "wiki-freshness": (p) => (p.get("slug") ? wikiFreshnessRef(p.get("slug")!) : null),
  "custom-dashboard": (p) => (p.get("id") ? customDashboardRef(p.get("id")!) : null),
  dashboard: (p) => (p.get("variant") === "visits" ? dashboardRef("visits") : null),
  "stream-settings": (p) => (p.get("stream") ? streamSettingsRef(p.get("stream")!) : null),
  "thread-settings": (p) => (p.get("thread") ? threadSettingsRef(p.get("thread")!) : null),
  "op-error": (p) => (p.get("id") ? opErrorRef(p.get("id")!) : null),
  "external-url": (p) => (p.get("url") ? externalUrlRef(p.get("url")!) : null),
};

function isRoute(name: string): name is RoutePageKind {
  return Object.prototype.hasOwnProperty.call(ROUTES, name);
}

/** The route name of a `page:` id (`page:diff-view?effort=e` → `diff-view`),
 *  or `null` when the id is not a page route. */
export function routeNameOf(tabId: string): RoutePageKind | null {
  if (!tabId.startsWith("page:")) return null;
  const { head } = splitParams(tabId.slice("page:".length));
  return isRoute(head) ? head : null;
}

/**
 * Reconstruct a full `TabRef` (with payload) from a tab id alone.
 *
 * Page-visit history rows persist only the id (`page_id`) and kind, not
 * the ref payload — so a naive `{ id, kind, payload: null }` rebuild
 * leaves payload-bearing pages broken (a `file` ref with no `path`
 * never opens; `wiki`/`work_item`/etc. render empty). Entity kinds
 * parse through the canonical grammar (so `:` inside
 * `work_item:oxplow:tsk42` is fine); routes parse their query params.
 *
 * Returns `null` for text that is not a ref, an unknown kind, or a route
 * whose params don't rebuild — a dead row the caller drops, never a tab
 * that renders blank.
 */
export function refFromTabId(id: string): TabRef | null {
  const canonical = parseRef(id);
  if (!canonical) return null;
  const kind: string = canonical.kind;
  switch (kind) {
    case "file":
      return fileRef(canonical.id, versionFromRev(canonical.rev));
    case "dir":
      return directoryRef(canonical.id);
    case "wiki":
      return wikiPageRef(canonical.id);
    case "work_item":
      return canonical.id.startsWith(OXPLOW_PROVIDER)
        ? taskRef(canonical.id.slice(OXPLOW_PROVIDER.length))
        : null;
    case "commit":
      return gitCommitRef(canonical.id);
    case "metric":
      return metricRef(canonical.id);
    case "lens":
      return lensRefFromTail(id.slice("lens:".length));
    case "page": {
      const { head, params } = splitParams(id.slice("page:".length));
      return isRoute(head) ? ROUTES[head](params) : null;
    }
    default:
      return null;
  }
}

/** The `PageKind` a tab id renders as: an entity ref's kind, or a
 *  route's name. `null` for text that is not a tab id. */
export function pageKindOf(tabId: string): PageKind | null {
  const canonical = parseRef(tabId);
  if (!canonical) return null;
  if (canonical.kind === "page") return routeNameOf(tabId);
  const entity: readonly string[] = ["file", "dir", "wiki", "work_item", "commit", "metric", "lens"];
  return entity.includes(canonical.kind) ? (canonical.kind as PageKind) : null;
}

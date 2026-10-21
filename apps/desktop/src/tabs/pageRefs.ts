/**
 * Helpers for constructing `TabRef` values consistently. Every tab id is
 * a canonical ref (`.context/resources.md`), built only here — never
 * hand-format an id.
 *
 * - An **entity** page's id is the entity's own ref: `file:<path>[@rev]`,
 *   `dir:<path>`, `wiki:<slug>`, `work_item:oxplow:tsk42`, `commit:<sha>`,
 *   `metric:<key>`, `lens:<ext>/<slug>[?params]`.
 * - A **declared page** (a manifest's `pages:` entry, not a thing in the
 *   graph) is `page:<id>[?params]` for oxplow's own extension (core-named,
 *   as its commands are: `page:tasks`, `page:settings?area=agents`) and
 *   `page:<extension>/<id>[?params]` for any other. Its kind is `page`;
 *   what draws it comes from its declaration. Pages never appear in
 *   `page_ref`.
 *
 * `refFromTabId` is the inverse of every constructor here: history,
 * bookmarks and the Go To page persist only the id, so each payload
 * must be rebuildable from it (the round-trip test in `pageRefs.test.ts`
 * covers every constructor).
 */

import type { PageKind, TabRef } from "./tabState.js";
import {
  WORKING,
  type Revision,
  revisionSlot,
  parseRevision,
  revisionFromSlot,
  shortRevisionLabel,
} from "../revision.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { agentSessionUri as agentSessionRefString, extPageUri as extPageRefString, formatUri, lensUri as lensRefString, metricUri as metricRefString, pageUri as pageRefString, parseUri, rawAfterType, uriOf, splitParams, withParams, type ParamValue } from "../uri/uri.js";

function canonicalId(kind: string, id: string, rev: string | null = null): string {
  return formatUri({ kind, id, rev, frag: null });
}

// Query params (`?k=v&k=v` after a route name or a lens id) are encoded and
// split by uri/uri.ts, the one place that spells a ref.

/** `tabId` with `params` added to its query (after any it has). */
export function withPageParams(tabId: string, params: Record<string, ParamValue>): string {
  return withParams(tabId, params);
}

/** What a page tab carries: whose page (`null` for oxplow's own, which are
 *  core-named), its id and the params its id carries. Nothing rides beside
 *  the id: what a page shows is in its params, what only one step of a tab
 *  keeps is that entry's history state. */
export interface PagePayload {
  extension: string | null;
  page: string;
  params: Record<string, string>;
}

function cleanParams(params?: Record<string, ParamValue>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(params ?? {})) if (v !== null && v !== undefined) out[k] = String(v);
  return out;
}

/** One of oxplow's own pages: `page:<id>[?params]`. */
export function pageTabRef(page: string, params?: Record<string, ParamValue>): TabRef {
  const clean = cleanParams(params);
  const payload: PagePayload = { extension: null, page, params: clean };
  return { id: pageRefString(page, Object.keys(clean).length ? clean : undefined), kind: "page", payload };
}

/** An extension's page: `page:<extension>/<id>[?params]` — `ref`, when it
 *  opens one of the extension's refs (P8.D7). */
export function extPageRef(extension: string, page: string, params?: Record<string, ParamValue>): TabRef {
  const clean = cleanParams(params);
  const payload: PagePayload = { extension, page, params: clean };
  return { id: extPageRefString(extension, page, Object.keys(clean).length ? clean : undefined), kind: "page", payload };
}

/** A page tab's payload; null for any other tab. */
export function pagePayload(ref: TabRef | null | undefined): PagePayload | null {
  return ref?.kind === "page" ? (ref.payload as PagePayload) : null;
}

/** Whether `ref` is the page `page` (oxplow's own unless `extension`). */
export function isPage(ref: TabRef | null | undefined, page: string, extension: string | null = null): boolean {
  const p = pagePayload(ref);
  return p !== null && p.page === page && p.extension === extension;
}

/** What kind of page a tab shows, as visits and bookmarks record it: an
 *  entity ref's kind, or a declared page's id (`tasks`, `acme/pr`). */
export function recordedPageKind(ref: TabRef): string {
  const p = pagePayload(ref);
  if (!p) return ref.kind;
  return p.extension ? `${p.extension}/${p.page}` : p.page;
}


// ---------------------------------------------------------------------------
// Entity pages
// ---------------------------------------------------------------------------

/**
 * Construct a file-tab ref. `version` is required: callers MUST
 * declare which version of the tree they want to view, even if the
 * answer is `WORKING` (the working tree). This rule exists because the
 * "implicit working tree" assumption is what made the duplication
 * scan show stale, mismatched line ranges in commit-target analysis.
 *
 * The id is the canonical ref: a working-tree file is `file:<path>`; a
 * historical view carries its revision (`file:<path>@git:HEAD`,
 * `@snap:<id>`) so it is a distinct tab from the working-tree view.
 */
export function fileRef(path: string, version: Revision = WORKING): TabRef {
  return { id: canonicalId("file", path, revisionSlot(version)), kind: "file", payload: { path, version } };
}

/** The working-tree file a tab id shows, or `null` when the id is not a
 *  file or pins a revision (a read-only viewer, not the editor's file). */
export function diskFilePath(tabId: string): string | null {
  const r = parseUri(tabId);
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

/** A work item's page, whichever list it's on: the tab id is its ref
 *  (`work_item:<provider>:<id>`). */
export function workItemTabRef(ref: string): TabRef {
  return { id: ref, kind: "work_item", payload: { ref } };
}

/** An agent session's tab (`agent_session:ses3`): its terminal.
 *  Pinned — it leads the strip and is never evicted; its tab follows
 *  the session's row (`reconcileSessionTabs`). */
export function agentSessionRef(sessionId: string): TabRef {
  return { id: agentSessionRefString(sessionId), kind: "agent_session", payload: { sessionId }, pinned: true };
}

/** Single git commit page. */
export function gitCommitRef(sha: string): TabRef {
  return { id: canonicalId("commit", sha), kind: "commit", payload: { sha } };
}

/** Open one metric's detail page, optionally scoped to an effort's window —
 *  the task-page metrics-panel drill-in ("In this effort" before→after +
 *  further exploration). Its own page kind (`metric`); Metrics and the
 *  dashboard tiles both navigate into it. */
export function metricRef(metricKey: string, params?: Record<string, SqlCell>): TabRef {
  const keys = Object.keys(params ?? {}).sort();
  if (!params || keys.length === 0) return { id: canonicalId("metric", metricKey), kind: "metric", payload: { metricKey } };
  const id = metricRefString(metricKey, Object.fromEntries(keys.map((k) => [k, params[k] === null ? "" : String(params[k])])));
  return { id, kind: "metric", payload: { metricKey, params } };
}

/** A lens page (`lens:<extension>/<slug>`): a user/agent-built query
 *  over the semantic layer from `.oxplow/extensions/`. `params` (e.g. a
 *  slot's `{ effort_id }`) ride in the id as a sorted query string, so
 *  the tab, its history and bookmarks reopen the lens with the same
 *  values. */
export function lensRef(lensId: string, params?: Record<string, SqlCell>): TabRef {
  const keys = Object.keys(params ?? {}).sort();
  if (!params || keys.length === 0) return { id: lensRefString(lensId), kind: "lens", payload: { lensId } };
  const id = lensRefString(lensId, Object.fromEntries(keys.map((k) => [k, params[k] === null ? "" : String(params[k])])));
  return { id, kind: "lens", payload: { lensId, params } };
}

/** A page's view params: what its ref carries besides what it is (a
 *  lens's params, a declared page's). None for a page without a view. */
export function viewParamsOf(ref: TabRef): Record<string, SqlCell> {
  if (ref.kind === "lens") return { ...((ref.payload as { params?: Record<string, SqlCell> } | null)?.params ?? {}) };
  if (ref.kind === "page") return { ...(pagePayload(ref)?.params ?? {}) };
  if (ref.kind === "metric") return { ...((ref.payload as { params?: Record<string, SqlCell> } | null)?.params ?? {}) };
  return {};
}

/** `ref` showing `params` as its view, in its own kind of ref: a lens's,
 *  a declared page's or a metric's. A ref without a view comes back as it
 *  is. */
export function withViewParams(ref: TabRef, params: Record<string, SqlCell>): TabRef {
  const values = params as Record<string, ParamValue>;
  if (ref.kind === "lens") return lensRef((ref.payload as { lensId: string }).lensId, params);
  if (ref.kind === "metric") return metricRef((ref.payload as { metricKey: string }).metricKey, params);
  const page = pagePayload(ref);
  if (!page) return ref;
  return page.extension === null ? pageTabRef(page.page, values) : extPageRef(page.extension, page.page, values);
}

/** Parse a metric id's raw `<key>?k=v` tail back into a ref. */
function metricRefFromTail(tail: string): TabRef {
  const { head, params } = splitParams(tail);
  const out: Record<string, SqlCell> = {};
  for (const [k, v] of params) out[k] = v === "" ? null : /^-?\d+(\.\d+)?$/.test(v) ? Number(v) : v;
  return metricRef(head, out);
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
// Oxplow's own pages (declared in oxplow-foundation's manifest)
// ---------------------------------------------------------------------------

/** The session picker: what a thread with no agent session shows
 *  (`NewSessionPage`). Closable, not pinned. */
export function newSessionRef(): TabRef {
  return pageTabRef("new-session");
}

/** A Settings area's page (`page:settings?area=agents`); the overview is
 *  `page:settings`. `section` is where it lands: an area's section id or
 *  an anchor inside one. Both are view params: one Settings page shows
 *  them. */
export function settingsRef(area: string | null = null, section: string | null = null): TabRef {
  const shown = area && area !== "overview" ? area : null;
  return pageTabRef("settings", { area: shown, section });
}

/** A diff page: its spec as params — `path`, `left`, `right`, `label`
 *  (the tab's suffix), `base` (the left side's name, when it isn't the
 *  version's own short label) and `line` (the line it lands on, a view
 *  param: a new line re-renders the same page). Literal text (a clipboard
 *  compare) isn't an address: it rides as the entry's history state
 *  (`diffTextOf`). */
export function diffRef(spec: DiffSpec): TabRef {
  return pageTabRef("diff", {
    path: spec.path,
    left: spec.leftVersion,
    right: spec.rightVersion,
    label: spec.labelOverride ?? null,
    base: spec.baseLabel && spec.baseLabel !== shortRevisionLabel(spec.leftVersion) ? spec.baseLabel : null,
    line: spec.revealLine && spec.revealLine > 0 ? spec.revealLine : null,
  });
}

/** A diff page's spec, from its params; null when they don't name a path
 *  and two versions. */
export function diffSpecOf(params: Record<string, string>): DiffSpec | null {
  const left = parseRevision(params.left ?? "");
  const right = parseRevision(params.right ?? "");
  if (!params.path || !left || !right) return null;
  const line = Number(params.line);
  return {
    path: params.path,
    leftVersion: left,
    rightVersion: right,
    baseLabel: params.base ?? shortRevisionLabel(left),
    labelOverride: params.label,
    revealLine: Number.isInteger(line) && line > 0 ? line : undefined,
  };
}

/** The history state a diff of literal text opens with, and reads back:
 *  the two sides' text, which no address holds. */
export interface DiffText {
  diffText: { left: string; right: string };
}

/** The two sides' literal text in a diff entry's history state; null for
 *  a diff of versions. */
export function diffTextOf(historyState: Record<string, unknown> | undefined): DiffText["diffText"] | null {
  const t = historyState?.diffText as { left?: unknown; right?: unknown } | undefined;
  return t && typeof t.left === "string" && typeof t.right === "string" ? { left: t.left, right: t.right } : null;
}

export interface DuplicateBlockPayload {
  leftPath: string;
  leftStart: number;
  leftEnd: number;
  /** Tree version the LEFT side was scanned against. The page reads
   *  file content at this version so highlighted line ranges match
   *  the displayed text — never silently substitutes the working
   *  tree. */
  leftVersion: Revision;
  rightPath: string;
  rightStart: number;
  rightEnd: number;
  rightVersion: Revision;
}

/**
 * Side-by-side view of a duplicate-block finding. Both ranges are
 * loaded at the version the scan ran against and highlighted; the
 * editors are scrolled so the two start lines line up at the top of
 * the viewport.
 */
export function duplicateBlockRef(payload: DuplicateBlockPayload): TabRef {
  return pageTabRef("duplicate-block", {
    left: payload.leftPath,
    left_lines: `${payload.leftStart}-${payload.leftEnd}`,
    left_at: payload.leftVersion,
    right: payload.rightPath,
    right_lines: `${payload.rightStart}-${payload.rightEnd}`,
    right_at: payload.rightVersion,
  });
}

/** A duplicate-block page's two sides, from its params; null when they
 *  don't name two ranges. */
export function duplicateBlockOf(params: Record<string, string>): DuplicateBlockPayload | null {
  const side = (path: string | undefined, lines: string | undefined, at: string | undefined) => {
    const m = /^(\d+)-(\d+)$/.exec(lines ?? "");
    const version = parseRevision(at ?? "");
    return path && m && version ? { path, start: Number(m[1]), end: Number(m[2]), version } : null;
  };
  const l = side(params.left, params.left_lines, params.left_at);
  const r = side(params.right, params.right_lines, params.right_at);
  if (!l || !r) return null;
  return {
    leftPath: l.path, leftStart: l.start, leftEnd: l.end, leftVersion: l.version,
    rightPath: r.path, rightStart: r.start, rightEnd: r.end, rightVersion: r.version,
  };
}

/** A symbol (`symbol:<path>/<id>`, `v_symbol.ref`, optionally pinned
 *  `@snap:`/`@git:`): opening it opens its file at the symbol (`openSymbol`). */
export function symbolRef(ref: string): TabRef {
  return { id: ref, kind: "symbol", payload: { ref } };
}

export function wikiFreshnessRef(slug: string): TabRef {
  return pageTabRef("wiki-freshness", { slug });
}

/** One user-created dashboard. The dashboard id (`dsh<n>`) rides in
 *  both the id and the payload so a history-restored tab resolves via
 *  `refFromTabId`. */
export function customDashboardRef(id: string): TabRef {
  return pageTabRef("custom-dashboard", { id });
}

/** The Dashboards index — the list of the user's custom dashboards: the
 *  foundation's `dashboards` lens page. */
export function dashboardsRef(): TabRef {
  return pageTabRef("dashboards");
}

/** The Metrics page — every catalogued definition with latest value, trend
 *  sparkline, capture branch, sample count.
 *
 *  `metricsIndexRef`, not `metricsRef`, to stay clearly distinct from
 *  {@link metricRef} (one metric's detail page). */
export function metricsIndexRef(): TabRef {
  return pageTabRef("metrics-recorded");
}

/** The global Comments inbox: every comment in the current stream. */
export function commentsRef(): TabRef {
  return pageTabRef("comments");
}

/** The Terminal page: a plain interactive shell rooted at the worktree dir. */
export function terminalRef(): TabRef {
  return pageTabRef("terminal");
}

/** The Hook Events page: the foundation's `hook-events` lens page. */
export function hookEventsRef(): TabRef {
  return pageTabRef("hook-events");
}

/**
 * Named ref helpers for the work pages. Mirrors the GitDashboard
 * pattern (`gitDashboardRef`, `uncommittedChangesRef`) so call sites
 * read as intent rather than as stringly-typed `pageTabRef("…")`.
 */
export function tasksRef(): TabRef {
  return pageTabRef("tasks");
}
/** The Done Work page: the foundation's `done-work` lens page. */
export function doneWorkRef(): TabRef {
  return pageTabRef("done-work");
}
export function backlogRef(): TabRef {
  return pageTabRef("backlog");
}

/** Git Dashboard — committed-history rollup page. */
export function gitDashboardRef(): TabRef {
  return pageTabRef("git-dashboard");
}

/**
 * A change framed as an explicit start→end diff, all rendered by
 * `DiffViewPage`. The things with an identity are entity pages whose id
 * is their canonical ref (P2.11); only an ad-hoc pair of endpoints is a
 * `diff-view` route:
 *
 * - **snapshot** (`snapshot:<N>`) — a single capture, framed as
 *   `[parent → N]` (its recorded parent).
 * - **effort** (`effort:<effN>`) — the effort's start/end snapshot
 *   bracket, carrying the task title + "in progress" state.
 * - **turn** (`turn:<trnN>`) — an agent turn's start/end snapshots:
 *   what the turn changed.
 * - **endpoints** (`page:diff-view?start=<rev>&end=<rev>`) — an explicit
 *   pair of revisions (`working`, `snap:<id>`, `git:<rev>`). `start =
 *   null` (`none`) diffs `end` against the empty tree (everything added).
 */
export type DiffViewPayload =
  | { mode: "snapshot"; snapshotId: number }
  | { mode: "effort"; effortId: string }
  | { mode: "turn"; turnId: string }
  | { mode: "endpoints"; start: Revision | null; end: Revision };

/** A single captured snapshot's page. Drill-in from the Local History
 *  dashboard, file version history, and snapshot backlinks. */
export function snapshotRef(snapshotId: number): TabRef {
  return {
    id: canonicalId("snapshot", String(snapshotId)),
    kind: "snapshot",
    payload: { mode: "snapshot", snapshotId },
  };
}

/** An effort's page — its start/end snapshot bracket as a diff. The
 *  'View diff' button on a completed effort points here. */
export function effortDiffRef(effortId: string): TabRef {
  return {
    id: canonicalId("effort", effortId),
    kind: "effort",
    payload: { mode: "effort", effortId },
  };
}

/** An agent turn's page (`trn<N>`) — what the turn changed. */
export function turnRef(turnId: string): TabRef {
  return { id: canonicalId("turn", turnId), kind: "turn", payload: { mode: "turn", turnId } };
}

/** Diff view between two revisions. `start = null` diffs `end` against
 *  the empty tree. */
export function endpointDiffRef(start: Revision | null, end: Revision): TabRef {
  return pageTabRef("diff-view", { start: start ?? "none", end });
}

/** A `diff-view` page's endpoints, from its params; null without an end. */
export function endpointsOf(params: Record<string, string>): DiffViewPayload | null {
  const end = parseRevision(params.end ?? "");
  if (!end) return null;
  const start = params.start ?? "none";
  return { mode: "endpoints", start: start === "none" ? null : parseRevision(start), end };
}

/** Uncommitted Changes — the working tree's changed files, commit form
 *  and the `uncommitted` lens slot. */
export function uncommittedChangesRef(): TabRef {
  return pageTabRef("uncommitted-changes");
}

/** The Go To page: the foundation's `go-to` lens page. */
export function goToRef(): TabRef {
  return pageTabRef("go-to");
}

/**
 * Form pages: a focused full-tab workspace for each form (new stream,
 * new task, stream and thread settings), matching `SettingsPage`.
 */

export interface NewTaskPayload {
  /** Optional pre-selected parent epic (its ref). */
  parentRef?: string | null;
}

export function newStreamRef(): TabRef {
  return pageTabRef("new-stream");
}

/** The id is stable (no params) so re-opening the page reuses the
 *  existing tab rather than stacking duplicates; the defaults are read
 *  on mount, so a history reopen starts from an empty form. Callers
 *  wanting different defaults should `closeTab` before opening with a
 *  new payload. */
export function newTaskRef(payload: NewTaskPayload = {}): TabRef {
  return pageTabRef("new-task", { parent: payload.parentRef ?? null });
}

export function streamSettingsRef(streamId: string): TabRef {
  return pageTabRef("stream-settings", { stream: streamId });
}

export function threadSettingsRef(threadId: string): TabRef {
  return pageTabRef("thread-settings", { thread: threadId });
}

/** Async-op error detail page. Scoped to the error id so each failure
 *  gets its own tab; closing it discards the view, not the store entry. */
/** The Alerts page: everything that needs the person. */
export function alertsRef(): TabRef {
  return pageTabRef("alerts");
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
  return pageTabRef("external-url", { url });
}

// ---------------------------------------------------------------------------
// Inverse: id → ref
// ---------------------------------------------------------------------------

/**
 * Reconstruct a full `TabRef` (with payload) from a tab id alone.
 *
 * Page-visit history rows persist only the id (`page_id`) and kind, not
 * the ref payload — so a naive `{ id, kind, payload: null }` rebuild
 * leaves payload-bearing pages broken (a `file` ref with no `path`
 * never opens; `wiki`/`work_item`/etc. render empty). Entity kinds
 * parse through the canonical grammar (so `:` inside
 * `work_item:oxplow:tsk42` is fine); pages keep their query params.
 *
 * Returns `null` for text that is not a ref or an unknown kind — a dead
 * row the caller drops. A page id always rebuilds; a page no extension
 * declares renders as "No such page".
 */
export function refFromTabId(id: string): TabRef | null {
  const canonical = parseUri(id);
  if (!canonical) return null;
  const kind: string = canonical.kind;
  switch (kind) {
    case "file":
      return fileRef(canonical.id, revisionFromSlot(canonical.rev));
    case "dir":
      return directoryRef(canonical.id);
    case "wiki":
      return wikiPageRef(canonical.id);
    case "work_item":
      return workItemTabRef(uriOf("work_item", canonical.id));
    case "commit":
      return gitCommitRef(canonical.id);
    case "metric":
      return metricRefFromTail(rawAfterType(id, "metric") ?? "");
    case "snapshot": {
      const n = Number(canonical.id);
      return Number.isInteger(n) ? snapshotRef(n) : null;
    }
    case "effort":
      return effortDiffRef(canonical.id);
    case "turn":
      return turnRef(canonical.id);
    case "symbol":
      return symbolRef(id);
    case "agent_session":
      return agentSessionRef(canonical.id);
    case "dashboard":
      return customDashboardRef(canonical.id);
    case "lens":
      return lensRefFromTail(rawAfterType(id, "lens") ?? "");
    case "page": {
      // Oxplow's own pages are `page:<id>`, another extension's
      // `page:<extension>/<id>`; what draws one comes from its declaration.
      const { head, params } = splitParams(rawAfterType(id, "page") ?? "");
      const values = Object.fromEntries(params.entries());
      const slash = head.indexOf("/");
      if (slash === -1) return head ? pageTabRef(head, values) : null;
      const extension = head.slice(0, slash);
      const page = head.slice(slash + 1);
      return extension && page && !page.includes("/") ? extPageRef(extension, page, values) : null;
    }
    default:
      // Not a tab id: a ref of another kind opens where its kind's page
      // says (`openRef`, `refOpener`), not here.
      return null;
  }
}

/** The `PageKind` a tab id renders as: an entity ref's kind, or `page`.
 *  `null` for text that is not a tab id. */
export function pageKindOf(tabId: string): PageKind | null {
  const canonical = parseUri(tabId);
  if (!canonical) return null;
  if (canonical.kind === "page") return "page";
  // The entity kinds that open as a page.
  const entity: readonly string[] = [
    "file",
    "dir",
    "wiki",
    "work_item",
    "commit",
    "metric",
    "lens",
    "snapshot",
    "effort",
    "turn",
    "symbol",
    "agent_session",
  ];
  return entity.includes(canonical.kind) ? (canonical.kind as PageKind) : null;
}

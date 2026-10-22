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
 *   as its commands are: `page:tasks`, `page:settings#area=agents`) and
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
import {
  agentSessionUri,
  commitUri,
  dirUri,
  effortUri,
  extPageUri,
  fileUri,
  formatUri,
  identityOf,
  keyParamOf,
  lensUri,
  viewParamsOfUri,
  withUriView,
  metricUri,
  pageUri,
  parseUri,
  snapshotUri,
  turnUri,
  typeOf,
  wikiUri,
  workItemUri,
  type ParamValue,
} from "../uri/uri.js";

// Params (`?k=v&k=v`, named, keys sorted) are encoded and split by
// uri/uri.ts, the one place that spells a URI.

/** `tabId` with `params` added to its params (a null or undefined value
 *  leaves one out). */
export function withPageParams(tabId: string, params: Record<string, ParamValue>): string {
  const u = parseUri(tabId);
  if (!u) return tabId;
  const merged = { ...u.params };
  for (const [k, v] of Object.entries(params)) if (v !== null && v !== undefined) merged[k] = String(v);
  return formatUri({ ...u, params: merged });
}

/** What a page tab carries: whose page (`null` for oxplow's own, which are
 *  core-named), its id and the params that say what it shows (its id's
 *  `?`). Nothing rides beside the id: how it shows it is the id's `#`
 *  view (`viewParamsOf`), what only one step of a tab keeps is that entry's
 *  history state. */
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

/** One of oxplow's own pages: `oxplow:oxplow/page?page=<id>[&params][#view]`. */
export function pageTabRef(page: string, params?: Record<string, ParamValue>, view?: Record<string, ParamValue>): TabRef {
  const clean = cleanParams(params);
  const payload: PagePayload = { extension: null, page, params: clean };
  return { id: pageUri(page, clean, view), kind: "page", payload };
}

/** An extension's page (`page=<extension>/<id>`) — `uri`, when it opens one
 *  of the extension's resources. */
export function extPageRef(
  extension: string,
  page: string,
  params?: Record<string, ParamValue>,
  view?: Record<string, ParamValue>,
): TabRef {
  const clean = cleanParams(params);
  const payload: PagePayload = { extension, page, params: clean };
  return { id: extPageUri(extension, page, clean, view), kind: "page", payload };
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
  return { id: fileUri(path, revisionSlot(version)), kind: "file", payload: { path, version } };
}

/** A file's tab landing on `line`: its `#lines=` view. */
export function fileRefAt(path: string, line: number | null, version: Revision = WORKING): TabRef {
  return withViewParams(fileRef(path, version), lineView(line));
}

/** The working-tree file a tab id shows, or `null` when the id is not a
 *  file or pins a revision (a read-only viewer, not the editor's file). */
export function diskFilePath(tabId: string): string | null {
  const r = parseUri(tabId);
  return r && typeOf(tabId) === "oxplow/file" && !("rev" in r.params) ? (r.params.path ?? null) : null;
}

export function directoryRef(path: string): TabRef {
  // Trailing slash is normalized away — `[[src/]]` and `[[src]]` (when
  // ever the parser admits the latter) collapse to one tab.
  const bare = path.replace(/\/+$/, "");
  return { id: dirUri(bare), kind: "dir", payload: { path: bare } };
}

export function wikiPageRef(slug: string): TabRef {
  return { id: wikiUri(slug), kind: "wiki", payload: { slug } };
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
  return { id: agentSessionUri(sessionId), kind: "agent_session", payload: { sessionId }, pinned: true };
}

/** Single git commit page. */
export function gitCommitRef(sha: string): TabRef {
  return { id: commitUri(sha), kind: "commit", payload: { sha } };
}

/** Open one metric's detail page, optionally scoped to an effort's window —
 *  the task-page metrics-panel drill-in ("In this effort" before→after +
 *  further exploration). Its own page kind (`metric`); Metrics and the
 *  dashboard tiles both navigate into it. */
export function metricRef(metricKey: string, view?: Record<string, SqlCell>): TabRef {
  return { id: metricUri(metricKey, {}, cellParams(view)), kind: "metric", payload: { metricKey } };
}

/** A lens page (`oxplow:oxplow/lens?lens=<extension>/<slug>`): a query over
 *  the semantic layer. `params` are its resource params (a slot's
 *  `{ effort_id }`: what it shows, in the id's `?`); `view` its `filter`
 *  and `selection` params (the id's `#`). Tabs, history and bookmarks
 *  reopen it from the id alone. */
export function lensRef(lensId: string, params?: Record<string, SqlCell>, view?: Record<string, SqlCell>): TabRef {
  const own = { ...(params ?? {}) };
  const id = lensUri(lensId, cellParams(own), cellParams(view));
  return { id, kind: "lens", payload: Object.keys(own).length > 0 ? { lensId, params: own } : { lensId } };
}

/** A ref's view: its id's `#` params, read as the params form reads them
 *  (an empty value is null, a number a number). */
export function viewParamsOf(ref: TabRef): Record<string, SqlCell> {
  return cellsOf(viewParamsOfUri(ref.id));
}

/** `ref`'s page (a lens's, a declared page's) naming `params` instead —
 *  another resource — in `view`: its key param kept, its payload rebuilt
 *  from the new id. */
export function withResourceParams(ref: TabRef, params: Record<string, SqlCell>, view: Record<string, SqlCell> = {}): TabRef {
  const u = parseUri(ref.id);
  const resource = typeOf(ref.id);
  if (!u || !resource) return ref;
  const key = keyParamOf(resource);
  const id = formatUri({ resource: u.resource, params: { ...cellParams(params), [key]: u.params[key] ?? "" }, view: cellParams(view) });
  return refFromTabId(id) ?? ref;
}

/** `ref` showing `view` (its id's `#`): the same page, whatever its kind. */
export function withViewParams(ref: TabRef, view: Record<string, SqlCell>): TabRef {
  return { ...ref, id: withUriView(ref.id, cellParams(view)) };
}

/** Values as a URI holds them: null as empty (a value set to nothing),
 *  undefined left out. */
function cellParams(values: Record<string, SqlCell | undefined> | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(values ?? {})) if (v !== undefined) out[k] = v === null ? "" : String(v);
  return out;
}

/** A URI's params read back: an empty value is null and a number is a
 *  number, as the params form reads them. */
function cellsOf(params: Record<string, string>, skip?: string): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = {};
  for (const [k, v] of Object.entries(params)) {
    if (k === skip) continue;
    out[k] = v === "" ? null : /^-?\d+(\.\d+)?$/.test(v) ? Number(v) : v;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Oxplow's own pages (declared in oxplow-foundation's manifest)
// ---------------------------------------------------------------------------

/** The session picker: what a thread with no agent session shows
 *  (`NewSessionPage`). Closable, not pinned. */
export function newSessionRef(): TabRef {
  return pageTabRef("new-session");
}

/** A Settings area's page (`page:settings#area=agents`); the overview is
 *  `page:settings`. `section` is where it lands: an area's section id or
 *  an anchor inside one. Both are view params: one Settings page shows
 *  them. */
export function settingsRef(area: string | null = null, section: string | null = null): TabRef {
  const shown = area && area !== "overview" ? area : null;
  return pageTabRef("settings", {}, { area: shown, section });
}

/** A diff page: its spec as params — `path`, `left`, `right`, `label`
 *  (the tab's suffix), `base` (the left side's name, when it isn't the
 *  version's own short label) and `line` (the line it lands on, a view
 *  param: a new line re-renders the same page). Literal text (a clipboard
 *  compare) isn't an address: it rides as the entry's history state
 *  (`diffTextOf`). */
export function diffRef(spec: DiffSpec): TabRef {
  return pageTabRef(
    "diff",
    {
      path: spec.path,
      left: spec.leftVersion,
      right: spec.rightVersion,
      label: spec.labelOverride ?? null,
      base: spec.baseLabel && spec.baseLabel !== shortRevisionLabel(spec.leftVersion) ? spec.baseLabel : null,
    },
    lineView(spec.revealLine),
  );
}

/** A diff page's spec, from its ref (the line it lands on is its view);
 *  null when its params don't name a path and two versions. */
export function diffSpecOf(ref: TabRef): DiffSpec | null {
  const params = pagePayload(ref)?.params ?? {};
  const left = parseRevision(params.left ?? "");
  const right = parseRevision(params.right ?? "");
  if (!params.path || !left || !right) return null;
  return {
    path: params.path,
    leftVersion: left,
    rightVersion: right,
    baseLabel: params.base ?? shortRevisionLabel(left),
    labelOverride: params.label,
    revealLine: lineOf(ref) ?? undefined,
  };
}

/** The view that lands on `line` (`#lines=12-12`); none without one. */
export function lineView(line: number | null | undefined): Record<string, string> {
  return line && line > 0 ? { lines: `${line}-${line}` } : {};
}

/** The line a ref's view lands on (the start of its `#lines=`), or null. */
export function lineOf(ref: TabRef): number | null {
  const start = Number((viewParamsOfUri(ref.id).lines ?? "").split("-")[0]);
  return Number.isInteger(start) && start > 0 ? start : null;
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
 * - **endpoints** (`page:diff-view#start=<rev>&end=<rev>`) — an explicit
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
    id: snapshotUri(snapshotId),
    kind: "snapshot",
    payload: { mode: "snapshot", snapshotId },
  };
}

/** An effort's page — its start/end snapshot bracket as a diff. The
 *  'View diff' button on a completed effort points here. */
export function effortDiffRef(effortId: string): TabRef {
  return {
    id: effortUri(effortId),
    kind: "effort",
    payload: { mode: "effort", effortId },
  };
}

/** An agent turn's page (`trn<N>`) — what the turn changed. */
export function turnRef(turnId: string): TabRef {
  return { id: turnUri(turnId), kind: "turn", payload: { mode: "turn", turnId } };
}

/** Diff view between two revisions. `start = null` diffs `end` against
 *  the empty tree. */
export function endpointDiffRef(start: Revision | null, end: Revision): TabRef {
  return pageTabRef("diff-view", {}, { start: start ?? "none", end });
}

/** A `diff-view` page's endpoints, from its view; null without an end. */
export function endpointsOf(ref: TabRef): DiffViewPayload | null {
  const view = viewParamsOfUri(ref.id);
  const end = parseRevision(view.end ?? "");
  if (!end) return null;
  const start = view.start ?? "none";
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
  const u = parseUri(id);
  if (!u) return null;
  const ref = refOfIdentity(u.params, typeOf(id), id);
  // Its view rides on the id as it came.
  return ref && Object.keys(u.view).length > 0 ? { ...ref, id: withUriView(ref.id, u.view) } : ref;
}

function refOfIdentity(p: Record<string, string>, resource: string | null, id: string): TabRef | null {
  switch (resource) {
    case "oxplow/file":
      return p.path ? fileRef(p.path, revisionFromSlot(p.rev ?? null)) : null;
    case "oxplow/dir":
      return p.path ? directoryRef(p.path) : null;
    case "oxplow/wiki":
      return p.slug ? wikiPageRef(p.slug) : null;
    case "oxplow/work_item":
      return p.provider && p.id ? workItemTabRef(workItemUri(p.provider, p.id)) : null;
    case "oxplow/commit":
      return p.sha ? gitCommitRef(p.sha) : null;
    case "oxplow/metric":
      return p.metric ? metricRef(p.metric) : null;
    case "oxplow/snapshot": {
      const n = Number(p.id);
      return Number.isInteger(n) ? snapshotRef(n) : null;
    }
    case "oxplow/effort":
      return p.id ? effortDiffRef(p.id) : null;
    case "oxplow/turn":
      return p.id ? turnRef(p.id) : null;
    case "oxplow/symbol":
      return symbolRef(identityOf(id) ?? id);
    case "oxplow/agent_session":
      return p.id ? agentSessionRef(p.id) : null;
    case "oxplow/dashboard":
      return p.id ? customDashboardRef(p.id) : null;
    case "oxplow/lens":
      return p.lens ? lensRef(p.lens, cellsOf(p, "lens")) : null;
    case "oxplow/page": {
      // Oxplow's own pages are `page=<id>`, another extension's
      // `page=<extension>/<id>`; what draws one comes from its declaration.
      const head = p.page ?? "";
      const { page: _page, ...values } = p;
      const slash = head.indexOf("/");
      if (slash === -1) return head ? pageTabRef(head, values) : null;
      const extension = head.slice(0, slash);
      const page = head.slice(slash + 1);
      return extension && page && !page.includes("/") ? extPageRef(extension, page, values) : null;
    }
    default:
      // Not a tab id: a URI of another resource opens where its resource's
      // page says (`openRef`, `refOpener`), not here.
      return null;
  }
}

/** The `PageKind` a tab id renders as: an entity ref's kind, or `page`.
 *  `null` for text that is not a tab id. */
export function pageKindOf(tabId: string): PageKind | null {
  const resource = typeOf(tabId);
  if (resource === null) return null;
  if (resource === "oxplow/page") return "page";
  // The resources that open as a page.
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
  const name = resource.slice("oxplow/".length);
  return resource.startsWith("oxplow/") && entity.includes(name) ? (name as PageKind) : null;
}

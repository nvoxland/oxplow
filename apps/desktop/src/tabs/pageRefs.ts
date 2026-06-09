/**
 * Helpers for constructing `TabRef` values consistently. Centralizing the
 * id format keeps cross-component links and ⌘K open-by-id stable.
 */

import type { TabRef } from "./tabState.js";
import { DISK, type FileVersion, versionIdFragment } from "../file-version.js";

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
 * Disk-version files use the legacy `file:<path>` id so existing
 * persistence and history continue to land on the same tab; non-disk
 * versions get a `:@<version>` suffix so a working-tree view and a
 * historical view of the same path are distinct tabs.
 */
export function fileRef(path: string, version: FileVersion = DISK): TabRef {
  const id =
    version.kind === "disk"
      ? `file:${path}`
      : `file:${path}:@${versionIdFragment(version)}`;
  return { id, kind: "file", payload: { path, version } };
}

export function directoryRef(path: string): TabRef {
  // Trailing slash is normalized away — `[[src/]]` and `[[src]]` (when
  // ever the parser admits the latter) collapse to one tab.
  const bare = path.replace(/\/+$/, "");
  return { id: `dir:${bare}`, kind: "directory", payload: { path: bare } };
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
  return { id: `wiki:${slug}`, kind: "wiki", payload: { slug } };
}

export function wikiFreshnessRef(slug: string): TabRef {
  return { id: `wiki-freshness:${slug}`, kind: "wiki-freshness", payload: { slug } };
}

export function taskRef(itemId: string): TabRef {
  return { id: `task:${itemId}`, kind: "task", payload: { itemId } };
}

export function findingRef(findingId: string): TabRef {
  return { id: `finding:${findingId}`, kind: "finding", payload: { findingId } };
}

export function effortCoverageRef(effortId: string): TabRef {
  return { id: `effort-coverage:${effortId}`, kind: "effort-coverage", payload: { effortId } };
}

export function indexRef(kind: "tasks" | "done-work" | "backlog" | "archived" | "wiki-index" | "files" | "comments" | "code-quality" | "local-history" | "local-history-full" | "local-history-by-commit-full" | "git-history" | "hook-events" | "subsystem-docs" | "terminal" | "settings"): TabRef {
  return { id: kind, kind, payload: null };
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

/** Detail page for a single snapshot (drill-in from the Local
 *  History dashboard's Recent Snapshots card). */
export function snapshotRef(snapshotId: number): TabRef {
  return {
    id: `snapshot:${snapshotId}`,
    kind: "snapshot",
    payload: { snapshotId },
  };
}

/**
 * Drilldown scope. `undefined` is "no scope" — show every changed
 * file. The host pages (commit, uncommitted) own the scope on their
 * own ref so a pivot click stays in-page rather than spawning a
 * separate analysis tab.
 */
export type ChangeAnalysisScope =
  | { kind: "ext"; value: string }
  | { kind: "dir"; value: string }
  | { kind: "status"; value: string };

/** Kept as an alias for callers that still talk in terms of
 *  `target` ("working" or a commit sha). New code should call the
 *  host ref directly. */
export type ChangeAnalysisTarget = "working" | string;

/** Uncommitted Changes — stats + analysis panel for the working
 *  tree. Optional drilldown scope (set when a pivot row is clicked
 *  from inside the page) keeps the user on the same tab while
 *  filtering. */
export function uncommittedChangesRef(scope?: ChangeAnalysisScope): TabRef {
  if (scope) {
    return {
      id: `uncommitted-changes:${scope.kind}:${scope.value}`,
      kind: "uncommitted-changes",
      payload: { scope },
    };
  }
  return { id: "uncommitted-changes", kind: "uncommitted-changes", payload: null };
}

/**
 * Convenience for "Change Analysis at <target>" call sites. Picks
 * the right host ref by target: working-tree → uncommittedChangesRef,
 * commit sha → gitCommitRef. The standalone "change-analysis" tab
 * kind no longer exists — drilldowns stay on the host page with a
 * scope set on its ref instead.
 */
export function changeAnalysisRef(
  target: ChangeAnalysisTarget,
  scope?: ChangeAnalysisScope,
): TabRef {
  return target === "working" ? uncommittedChangesRef(scope) : gitCommitRef(target, scope);
}

/** Single git commit page. Optional drilldown scope is folded into
 *  the same ref so pivot clicks stay on the commit page rather than
 *  navigating to a separate analysis tab. */
export function gitCommitRef(sha: string, scope?: ChangeAnalysisScope): TabRef {
  if (scope) {
    return {
      id: `git-commit:${sha}:${scope.kind}:${scope.value}`,
      kind: "git-commit",
      payload: { sha, scope },
    };
  }
  return { id: `git-commit:${sha}`, kind: "git-commit", payload: { sha } };
}

export type DashboardKind = "planning" | "review" | "quality" | "visits";

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
 * never opens; `wiki`/`task`/etc. render empty). Each payload-bearing
 * scheme parses its key out of the id here. Index/dashboard kinds carry
 * no payload, so the id IS the kind and the fallback is fine.
 */
export function refFromTabId(id: string): TabRef {
  const colon = id.indexOf(":");
  const scheme = colon === -1 ? id : id.slice(0, colon);
  const rest = colon === -1 ? "" : id.slice(colon + 1);
  switch (scheme) {
    case "file": {
      // Disk file id is `file:<path>`; a versioned-viewer id appends
      // `:@<frag>`. Strip that so history reopens the disk file.
      const at = rest.lastIndexOf(":@");
      return fileRef(at === -1 ? rest : rest.slice(0, at));
    }
    case "wiki":
      return wikiPageRef(rest);
    case "wiki-freshness":
      return wikiFreshnessRef(rest);
    case "task":
      return taskRef(rest);
    case "finding":
      return findingRef(rest);
    case "directory":
      return directoryRef(rest);
    case "snapshot": {
      const n = Number(rest);
      return Number.isFinite(n) ? snapshotRef(n) : { id, kind: "snapshot", payload: null };
    }
    case "git-commit":
      // Id may carry a `:scope:value` suffix; the bare sha reopens the
      // full-commit view, which is the right default from history.
      return gitCommitRef(rest.split(":")[0] ?? rest);
    case "external-url":
      return externalUrlRef(rest);
    case "op-error":
      return opErrorRef(rest);
    default:
      // Index/dashboard kinds (`tasks`, `files`, `git-dashboard`, …) and
      // any unknown scheme: no payload needed; the id is the kind.
      return { id, kind: scheme as TabRef["kind"], payload: null };
  }
}

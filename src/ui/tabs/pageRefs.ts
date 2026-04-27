/**
 * Helpers for constructing `TabRef` values consistently. Centralizing the
 * id format keeps cross-component links and ⌘K open-by-id stable.
 */

import type { TabRef } from "./tabState.js";

export function agentRef(): TabRef {
  return { id: "agent", kind: "agent", payload: null };
}

export function fileRef(path: string): TabRef {
  return { id: `file:${path}`, kind: "file", payload: { path } };
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

export function noteRef(slug: string): TabRef {
  return { id: `note:${slug}`, kind: "note", payload: { slug } };
}

export function workItemRef(itemId: string): TabRef {
  return { id: `wi:${itemId}`, kind: "work-item", payload: { itemId } };
}

export function findingRef(findingId: string): TabRef {
  return { id: `finding:${findingId}`, kind: "finding", payload: { findingId } };
}

export function indexRef(kind: "plan-work" | "done-work" | "backlog" | "archived" | "notes-index" | "files" | "code-quality" | "local-history" | "git-history" | "hook-events" | "subsystem-docs" | "settings"): TabRef {
  return { id: kind, kind, payload: null };
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
export function planWorkRef(): TabRef {
  return indexRef("plan-work");
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

/** Uncommitted Changes — stats + tree view of working-tree changes. */
export function uncommittedChangesRef(): TabRef {
  return { id: "uncommitted-changes", kind: "uncommitted-changes", payload: null };
}

/** Single git commit page — bookmark-/history-friendly version of the
 *  commit slideover. Id format: `git-commit:<sha>`. */
export function gitCommitRef(sha: string): TabRef {
  return { id: `git-commit:${sha}`, kind: "git-commit", payload: { sha } };
}

export type DashboardKind = "planning" | "review" | "quality";

export function dashboardRef(variant: DashboardKind): TabRef {
  return { id: `dashboard:${variant}`, kind: "dashboard", payload: { variant } };
}

/**
 * Form pages introduced by phase 5e. These replace the legacy modal
 * dialogs (NewStreamModal / NewWorkItemModal / Stream-Thread settings)
 * with a focused full-tab workspace, matching `SettingsPage`.
 */

export interface NewWorkItemPayload {
  /** Optional pre-selected parent epic id. */
  parentId?: string | null;
  /** Optional default category (carried forward by "Save and Another"). */
  initialCategory?: string | null;
  /** Optional default priority. */
  initialPriority?: string | null;
  /** When present, the page renders in edit mode against this item: it
   *  loads the existing values, retitles the page to "Edit work item",
   *  and submits via `updateWorkItem` instead of `createWorkItem`. */
  editingItemId?: string | null;
}

export function newStreamRef(): TabRef {
  return { id: "new-stream", kind: "new-stream", payload: null };
}

export function newWorkItemRef(payload: NewWorkItemPayload = {}): TabRef {
  // Use a stable id so re-opening the page reuses the existing tab
  // rather than stacking duplicates. "Save and Another" relies on the
  // form re-mounting in place; the page reads its initial values on
  // mount, so callers wanting different defaults should `closeTab`
  // before opening with new payload.
  //
  // Edit-mode tabs use an item-scoped id so a user can have several
  // edit tabs open at once (one per item) without colliding with the
  // create tab.
  if (payload.editingItemId) {
    return {
      id: `new-work-item:edit:${payload.editingItemId}`,
      kind: "new-work-item",
      payload,
    };
  }
  return { id: "new-work-item", kind: "new-work-item", payload };
}

export function streamSettingsRef(streamId: string): TabRef {
  return { id: `stream-settings:${streamId}`, kind: "stream-settings", payload: { streamId } };
}

export function threadSettingsRef(threadId: string): TabRef {
  return { id: `thread-settings:${threadId}`, kind: "thread-settings", payload: { threadId } };
}

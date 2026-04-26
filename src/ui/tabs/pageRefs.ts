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

export function indexRef(kind: "all-work" | "notes-index" | "files" | "code-quality" | "local-history" | "git-history" | "subsystem-docs" | "settings" | "start"): TabRef {
  return { id: kind, kind, payload: null };
}

export type DashboardKind = "planning" | "review" | "quality";

export function dashboardRef(variant: DashboardKind): TabRef {
  return { id: `dashboard:${variant}`, kind: "dashboard", payload: { variant } };
}

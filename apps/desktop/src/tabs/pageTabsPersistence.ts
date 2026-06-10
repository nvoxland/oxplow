// localStorage persistence for the page-tab layout: per-thread tab
// lists, per-tab back/forward history, the diff-spec registry, open
// file sessions (paths only), and the last-active center tab.
//
// Extracted from App.tsx so the read-side coercions (corrupt JSON,
// legacy blob shapes) are unit-testable without mounting the shell.
// Every reader is total: parse failures and shape mismatches degrade
// to "nothing persisted", never a throw.

import type { TabRef } from "./tabState.js";
import type { NavSiblings } from "./PageNavigationContext.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import type { FileSessionState } from "../editor-session.js";
import { logUi } from "../logger.js";

export const FILE_SESSIONS_STORAGE_KEY = "oxplow.layout.v1.fileSessions";
export const CENTER_ACTIVE_STORAGE_KEY = "oxplow.layout.v1.centerActive";
export const THREAD_TABS_STORAGE_KEY = "oxplow.layout.v1.threadPageTabs";
export const THREAD_HISTORY_STORAGE_KEY = "oxplow.layout.v1.threadPageHistory";
export const DIFF_SPECS_STORAGE_KEY = "oxplow.layout.v1.diffSpecs";

/** A single back/forward stack frame. Stores both the ref and the
 *  siblings record from when that page was active, so going back
 *  restores the originating list's prev/next chain instead of
 *  dropping it. */
export type HistoryFrame = { ref: TabRef; siblings: NavSiblings | null };
export type ThreadHistory = Record<
  string,
  Record<string, { back: HistoryFrame[]; forward: HistoryFrame[]; siblings: NavSiblings | null }>
>;

/** Read persisted per-thread tab lists. Returns an empty record on
 *  parse failure or absence — the user lands with no page tabs. */
export function readPersistedThreadPageTabs(): Record<string, TabRef[]> {
  try {
    const raw = window.localStorage.getItem(THREAD_TABS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    const out: Record<string, TabRef[]> = {};
    for (const [threadId, refs] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof threadId !== "string" || !Array.isArray(refs)) continue;
      const clean = refs.filter((r): r is TabRef =>
        !!r && typeof r === "object" && typeof (r as TabRef).id === "string" && typeof (r as TabRef).kind === "string",
      );
      // Dedupe by id at read time so any pre-existing corrupted
      // state (the duplicate-git-dashboard bug) self-heals on
      // next launch. First occurrence wins so order is preserved.
      const seen = new Set<string>();
      const deduped: TabRef[] = [];
      for (const r of clean) {
        if (seen.has(r.id)) continue;
        seen.add(r.id);
        deduped.push(r);
      }
      if (deduped.length > 0) out[threadId] = deduped;
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to parse persisted threadPageTabs", { error: String(err) });
    return {};
  }
}

export function writePersistedThreadPageTabs(tabs: Record<string, TabRef[]>): void {
  try {
    // Drop empty thread entries to keep the blob small.
    const out: Record<string, TabRef[]> = {};
    for (const [threadId, refs] of Object.entries(tabs)) {
      if (refs.length > 0) out[threadId] = refs;
    }
    window.localStorage.setItem(THREAD_TABS_STORAGE_KEY, JSON.stringify(out));
  } catch (err) {
    logUi("warn", "failed to write persisted threadPageTabs", { error: String(err) });
  }
}

export function readPersistedThreadPageHistory(): ThreadHistory {
  try {
    const raw = window.localStorage.getItem(THREAD_HISTORY_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    // Persisted blob may be the older shape (TabRef[] for back/
    // forward) — coerce on the fly so old data still restores.
    const out: ThreadHistory = {};
    for (const [threadId, perThread] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof threadId !== "string" || !perThread || typeof perThread !== "object") continue;
      const inner: ThreadHistory[string] = {};
      for (const [tabId, raw] of Object.entries(perThread as Record<string, unknown>)) {
        if (!raw || typeof raw !== "object") continue;
        const entry = raw as { back?: unknown; forward?: unknown; siblings?: unknown };
        const coerce = (arr: unknown): HistoryFrame[] => {
          if (!Array.isArray(arr)) return [];
          return arr.map((item) => {
            if (item && typeof item === "object" && "ref" in (item as object)) {
              return item as HistoryFrame;
            }
            return { ref: item as TabRef, siblings: null };
          });
        };
        inner[tabId] = {
          back: coerce(entry.back),
          forward: coerce(entry.forward),
          siblings: (entry.siblings ?? null) as ThreadHistory[string][string]["siblings"],
        };
      }
      out[threadId] = inner;
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to parse persisted threadPageHistory", { error: String(err) });
    return {};
  }
}

export function writePersistedThreadPageHistory(history: ThreadHistory): void {
  try {
    window.localStorage.setItem(THREAD_HISTORY_STORAGE_KEY, JSON.stringify(history));
  } catch (err) {
    logUi("warn", "failed to write persisted threadPageHistory", { error: String(err) });
  }
}

export function readPersistedDiffSpecs(): Array<{ id: string; spec: DiffSpec }> {
  try {
    const raw = window.localStorage.getItem(DIFF_SPECS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: Array<{ id: string; spec: DiffSpec }> = [];
    for (const entry of parsed) {
      if (!entry || typeof entry !== "object") continue;
      const id = (entry as { id?: unknown }).id;
      const rawSpec = (entry as { spec?: unknown }).spec;
      if (typeof id !== "string" || !rawSpec || typeof rawSpec !== "object") continue;
      const s = rawSpec as Record<string, unknown>;
      // Coerce pre-versioning persisted specs (leftRef + rightKind)
      // into the new (leftVersion, rightVersion) shape. Existing
      // tabs survive a restart without losing their target.
      let leftVersion = (s.leftVersion ?? null) as DiffSpec["leftVersion"] | null;
      let rightVersion = (s.rightVersion ?? null) as DiffSpec["rightVersion"] | null;
      if (!leftVersion) {
        const lr = typeof s.leftRef === "string" ? s.leftRef : null;
        leftVersion = lr ? { kind: "ref", ref: lr } : { kind: "disk" };
      }
      if (!rightVersion) {
        const rk = s.rightKind;
        if (rk === "working") rightVersion = { kind: "disk" };
        else if (rk && typeof rk === "object" && typeof (rk as { ref?: unknown }).ref === "string") {
          rightVersion = { kind: "ref", ref: (rk as { ref: string }).ref };
        } else {
          rightVersion = { kind: "disk" };
        }
      }
      out.push({
        id,
        spec: {
          path: typeof s.path === "string" ? s.path : "",
          leftVersion,
          rightVersion,
          baseLabel: typeof s.baseLabel === "string" ? s.baseLabel : "",
          labelOverride: typeof s.labelOverride === "string" ? s.labelOverride : undefined,
          revealLine: typeof s.revealLine === "number" ? s.revealLine : undefined,
        },
      });
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to parse persisted diff specs", { error: String(err) });
    return [];
  }
}

export function writePersistedDiffSpecs(specs: Array<{ id: string; spec: DiffSpec }>): void {
  try {
    // Drop clipboard / synthetic specs that carry inline content too
    // large to persist comfortably; their `leftContent` / `rightContent`
    // are runtime-only. Keep ref-based diffs (fromRef/toRef paths) which
    // can be re-resolved on boot by reading the git refs.
    const persistable = specs.filter((s) => !s.spec.leftContent && !s.spec.rightContent);
    window.localStorage.setItem(DIFF_SPECS_STORAGE_KEY, JSON.stringify(persistable));
  } catch (err) {
    logUi("warn", "failed to write persisted diff specs", { error: String(err) });
  }
}

export function readPersistedFileSessionPaths(): Record<string, string[]> {
  try {
    const raw = window.localStorage.getItem(FILE_SESSIONS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    const out: Record<string, string[]> = {};
    for (const [streamId, paths] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof streamId !== "string") continue;
      if (!Array.isArray(paths)) continue;
      const clean = paths.filter((p): p is string => typeof p === "string");
      if (clean.length > 0) out[streamId] = clean;
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to parse persisted file sessions", { error: String(err) });
    return {};
  }
}

export function writePersistedFileSessionPaths(sessions: Record<string, FileSessionState>): void {
  try {
    const out: Record<string, string[]> = {};
    for (const [streamId, session] of Object.entries(sessions)) {
      if (session.openOrder.length > 0) out[streamId] = session.openOrder;
    }
    window.localStorage.setItem(FILE_SESSIONS_STORAGE_KEY, JSON.stringify(out));
  } catch {}
}

export function readPersistedCenterActive(): string | null {
  try {
    const raw = window.localStorage.getItem(CENTER_ACTIVE_STORAGE_KEY);
    return typeof raw === "string" && raw.length > 0 ? raw : null;
  } catch {
    return null;
  }
}

export function writePersistedCenterActive(value: string): void {
  try {
    window.localStorage.setItem(CENTER_ACTIVE_STORAGE_KEY, value);
  } catch {}
}

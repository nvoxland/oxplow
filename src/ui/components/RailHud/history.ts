import { useEffect, useState } from "react";
import type { TabRef } from "../../tabs/tabState.js";

export interface HistoryEntry {
  ref: TabRef;
  label: string;
  t: number;
}

const STORAGE_KEY = "oxplow.railHistory";
const MAX_ENTRIES = 25;
const LISTENERS = new Set<() => void>();

function readStorage(): HistoryEntry[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (e): e is HistoryEntry =>
        e && typeof e === "object" && e.ref && typeof e.ref.id === "string" && typeof e.label === "string",
    );
  } catch {
    return [];
  }
}

function writeStorage(entries: HistoryEntry[]) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(entries));
  } catch {}
  LISTENERS.forEach((l) => l());
}

/**
 * Record a page visit. Most-recent-first ordering, deduplicated by
 * ref.id, capped at MAX_ENTRIES. Skips agent tabs and unstable refs
 * (callers can pre-filter if needed).
 */
export function recordHistoryVisit(ref: TabRef, label: string): void {
  if (!ref?.id || !label) return;
  const entries = readStorage();
  const filtered = entries.filter((e) => e.ref.id !== ref.id);
  filtered.unshift({ ref, label, t: Date.now() });
  writeStorage(filtered.slice(0, MAX_ENTRIES));
}

export function clearHistory(): void {
  writeStorage([]);
}

/** Subscribe to history changes; returns the live list. */
export function useHistory(): HistoryEntry[] {
  const [entries, setEntries] = useState<HistoryEntry[]>(() => readStorage());
  useEffect(() => {
    const listener = () => setEntries(readStorage());
    LISTENERS.add(listener);
    return () => {
      LISTENERS.delete(listener);
    };
  }, []);
  return entries;
}

/**
 * Derive a label for a ref. App-level callers should pass an explicit
 * label (work item title, note title, etc.); this is the fallback for
 * static pages and files.
 */
export function deriveDefaultLabel(ref: TabRef): string {
  switch (ref.kind) {
    case "file": {
      const path = (ref.payload as { path?: string } | null)?.path ?? "";
      return path.split("/").pop() ?? path ?? "File";
    }
    case "plan-work": return "Plan work";
    case "done-work": return "Done work";
    case "backlog": return "Backlog";
    case "archived": return "Archived";
    case "notes-index": return "Notes";
    case "files": return "Files";
    case "code-quality": return "Code quality";
    case "local-history": return "Local history";
    case "git-history": return "Git history";
    case "git-dashboard": return "Git dashboard";
    case "git-commit": return "Git commit";
    case "uncommitted-changes": return "Uncommitted";
    case "hook-events": return "Hook events";
    case "subsystem-docs": return "Subsystem docs";
    case "settings": return "Settings";
    case "stream-settings": return "Stream settings";
    case "thread-settings": return "Thread settings";
    case "new-stream": return "New stream";
    case "new-work-item": return "New work item";
    case "dashboard": {
      const variant = (ref.payload as { variant?: string } | null)?.variant ?? "";
      return variant ? `Dashboard: ${variant}` : "Dashboard";
    }
    case "work-item": {
      const id = (ref.payload as { itemId?: string } | null)?.itemId ?? ref.id;
      return id;
    }
    case "note": {
      const slug = (ref.payload as { slug?: string } | null)?.slug ?? ref.id;
      return slug;
    }
    default:
      return ref.id;
  }
}

/** Test-only reset. */
export function _resetHistoryForTests(): void {
  if (typeof window !== "undefined") {
    try { window.localStorage.removeItem(STORAGE_KEY); } catch {}
  }
  LISTENERS.forEach((l) => l());
}

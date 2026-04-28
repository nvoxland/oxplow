import type { TabRef } from "../../tabs/tabState.js";

/**
 * Derive a default display label for a TabRef. App-level callers should
 * pass an explicit label when richer context is available (work item
 * title, note title, etc.); this is the fallback for static pages and
 * files.
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

/**
 * Ref kinds that should NOT be recorded as page visits. The agent
 * terminal is always-present, and creation pages have throwaway ids.
 */
export const NON_TRACKED_KINDS: ReadonlySet<string> = new Set([
  "agent",
  "new-stream",
  "new-work-item",
]);

/** Kinds excluded from the rail History display (still recorded for analytics). */
export const RAIL_HISTORY_EXCLUDE_KINDS: string[] = [
  "agent",
  "new-stream",
  "new-work-item",
  "diff",
  "git-commit",
];

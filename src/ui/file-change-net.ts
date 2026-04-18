import type { BatchFileChange, FileChangeKind } from "./api.js";

/**
 * Collapse a list of raw file-change rows for any path into one row per path
 * representing the net effect at the end of the scope (turn or batch):
 *   - created then deleted (no later create) → drop (transient; hides
 *     editor/tool temp files that tools of any kind create then remove)
 *   - deleted then created → "updated" (replaced in place)
 *   - ends on "created" with no prior "deleted" → "created"
 *   - ends on "deleted" with no prior "created" → "deleted"
 *   - otherwise → "updated"
 *
 * The most recent row is returned so the UI can show the final source / tool.
 */
export function netFileChanges(changes: BatchFileChange[]): BatchFileChange[] {
  const byPath = new Map<string, BatchFileChange[]>();
  for (const change of changes) {
    const list = byPath.get(change.path) ?? [];
    list.push(change);
    byPath.set(change.path, list);
  }
  const out: BatchFileChange[] = [];
  for (const list of byPath.values()) {
    list.sort((a, b) => a.created_at.localeCompare(b.created_at));
    const first = list[0]!;
    const last = list[list.length - 1]!;
    const hasCreated = list.some((c) => c.change_kind === "created");
    const hasDeleted = list.some((c) => c.change_kind === "deleted");
    if (first.change_kind === "created" && last.change_kind === "deleted") continue;
    let netKind: FileChangeKind;
    if (last.change_kind === "created") netKind = "created";
    else if (last.change_kind === "deleted" && !hasCreated) netKind = "deleted";
    else if (hasDeleted && hasCreated) netKind = "updated";
    else netKind = last.change_kind;
    out.push({ ...last, change_kind: netKind });
  }
  return out.sort((a, b) => b.created_at.localeCompare(a.created_at));
}

export function fileChangeKindColor(kind: FileChangeKind): string {
  switch (kind) {
    case "created": return "var(--accent)";
    case "deleted": return "#d66";
    default: return "var(--fg)";
  }
}

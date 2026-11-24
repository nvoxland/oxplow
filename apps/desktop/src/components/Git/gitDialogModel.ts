/// What Git ▸'s dialogs list (`CompareWithDialog`, `RollbackDialog`),
/// as data: pure, tested without a window.
import type { GitStatusEntry } from "../../gitMenu.js";

export interface CompareData {
  /** `vcs.branches`: a remote one has its `remote`. */
  branches: { name: string; remote: string | null }[];
  /** `v_tag` names. */
  tags: string[];
  /** The file's recent commits (`vcs.log { path }`). */
  commits: { id: string; short_id: string; subject: string; author: string; time: number }[];
}

export interface CompareChoice {
  section: "Branches" | "Tags" | "Commits" | "Revision";
  label: string;
  /** What `git:` names on the diff's older side. */
  rev: string;
  detail?: string;
}

/** The choices `query` keeps: branches (local, then remote; not a
 *  remote's `HEAD`), tags, then the file's commits by sha or subject; and,
 *  when typed text matches none of them, the text itself (`HEAD~3`, an
 *  older sha). */
export function compareChoices(query: string, data: CompareData): CompareChoice[] {
  const q = query.trim().toLowerCase();
  const hit = (...texts: string[]) => !q || texts.some((t) => t.toLowerCase().includes(q));
  const branches = [...data.branches]
    .filter((b) => !(b.remote && b.name === "HEAD"))
    .sort((a, b) => Number(!!a.remote) - Number(!!b.remote))
    .map((b) => (b.remote ? `${b.remote}/${b.name}` : b.name));
  const out: CompareChoice[] = [
    ...branches.filter((b) => hit(b)).map((b): CompareChoice => ({ section: "Branches", label: b, rev: b })),
    ...data.tags.filter((t) => hit(t)).map((t): CompareChoice => ({ section: "Tags", label: t, rev: t })),
    ...data.commits
      .filter((c) => hit(c.id, c.subject))
      .map((c): CompareChoice => ({
        section: "Commits",
        label: `${c.short_id} ${c.subject}`,
        rev: c.id,
        detail: c.author,
      })),
  ];
  if (q && out.length === 0) out.push({ section: "Revision", label: `Use ${query.trim()}`, rev: query.trim() });
  return out;
}

export interface RollbackRow {
  /** Its folder ("" at the root). */
  folder: string;
  path: string;
  name: string;
  status: GitStatusEntry["status"];
  checked: boolean;
  /** Rolling it back deletes it (it is untracked). */
  deletes: boolean;
}

/** The stream's changes as the checklist's rows, by folder then name,
 *  those in `checked` checked. */
export function rollbackRows(changes: GitStatusEntry[], checked: string[]): RollbackRow[] {
  const on = new Set(checked);
  return changes
    .map((e): RollbackRow => {
      const path = e.path.replace(/\/+$/, "");
      const at = path.lastIndexOf("/");
      return {
        folder: at === -1 ? "" : path.slice(0, at),
        path,
        name: at === -1 ? path : path.slice(at + 1),
        status: e.status,
        checked: on.has(path),
        deletes: e.status === "untracked",
      };
    })
    .sort((a, b) => a.folder.localeCompare(b.folder) || a.name.localeCompare(b.name));
}

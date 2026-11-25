/// Git ▸: the one submenu a file's or a folder's right-click menu gets for
/// what git does with that path — never the repository's operations (push,
/// pull, branches): Annotate with Git Blame, Show History, Show Diff,
/// Compare with…, Rollback…, Add to .gitignore, those that apply to the
/// path now. An extension's commands about the path in the `Git` group
/// join after them (`withLinkOffers`, `openIn.ts`). `.context/git-integration.md`
/// "Git ▸ on a file".
import type { MenuItem } from "./menu.js";
import { gitRevision, WORKING } from "./revision.js";
import { diffRef, fileRef, pageTabRef, uncommittedChangesRef, withViewParams } from "./tabs/pageRefs.js";
import { paramOf, typeOf } from "./uri/uri.js";

/** A working-copy path of the stream, relative to its worktree. */
export type GitTarget = { kind: "file" | "dir"; path: string };

/** A changed path, as `vcs.status` says it (`entries`). */
export interface GitStatusEntry {
  path: string;
  status: "added" | "modified" | "deleted" | "renamed" | "untracked" | "conflicted";
}

/** What the items do, in this window. */
export interface GitMenuEnv {
  /** Open a ref (a page, a file in the editor) in a tab. */
  openRef(ref: string): void;
  /** The Compare with… dialog, for a file. */
  compareWith(path: string): void;
  /** The Rollback… dialog over `paths` (the changed ones), `checked` checked. */
  rollback(paths: string[], checked: string[]): void;
  /** Add the path to `.gitignore`. */
  ignore(path: string): void;
}

/** The view param that opens a file's editor with its blame shown. */
export const BLAME_VIEW = "oxplow.blame";

/** The Git target `ref` names: a working-copy file or folder (not the
 *  root) on the local filesystem. Null for anything else. */
export function gitTargetOf(ref: string): GitTarget | null {
  const type = typeOf(ref);
  if (type !== "oxplow/file" && type !== "oxplow/dir") return null;
  const rev = paramOf(ref, "rev");
  if ((rev && rev !== WORKING) || paramOf(ref, "filesystem")) return null;
  const path = (paramOf(ref, "path") ?? "").replace(/^\/+/, "").replace(/\/+$/, "");
  if (!path) return null;
  return { kind: type === "oxplow/file" ? "file" : "dir", path };
}

const under = (dir: string, path: string) => path === dir || path.startsWith(`${dir}/`);
const clean = (path: string) => path.replace(/\/+$/, "");

/** The Git ▸ item for `target` given the stream's changes (`status`), with
 *  `extra` (an extension's) after oxplow's items. Null when none applies. */
export function gitMenu(target: GitTarget, status: GitStatusEntry[], env: GitMenuEnv, extra: MenuItem[] = []): MenuItem | null {
  const { path } = target;
  const mine = status.filter((e) => (target.kind === "file" ? clean(e.path) === path : under(path, clean(e.path))));
  const untracked = mine.length > 0 && mine.every((e) => e.status === "untracked");
  const changed = mine.length > 0;
  const all = status.map((e) => clean(e.path));
  const items: MenuItem[] = [];
  const item = (id: string, label: string, run: () => void): MenuItem => ({ id: `git.${id}`, label, enabled: true, run });
  if (!untracked) {
    if (target.kind === "file") {
      items.push(item("blame", "Annotate with Git Blame", () => env.openRef(withViewParams(fileRef(path), { [BLAME_VIEW]: "1" }).id)));
    }
    items.push(item("history", "Show History", () => env.openRef(pageTabRef("git-history", { path }).id)));
    if (changed) {
      items.push(
        item("diff", "Show Diff", () =>
          env.openRef(
            target.kind === "file"
              ? diffRef({ path, leftVersion: gitRevision("HEAD"), rightVersion: WORKING, baseLabel: "HEAD" }).id
              : uncommittedChangesRef().id,
          ),
        ),
      );
    }
    if (target.kind === "file") items.push(item("compare", "Compare with…", () => env.compareWith(path)));
  }
  if (changed) items.push(item("rollback", "Rollback…", () => env.rollback(all, mine.map((e) => clean(e.path)))));
  if (untracked) items.push(item("ignore", "Add to .gitignore", () => env.ignore(path)));
  const submenu = [...items, ...extra];
  return submenu.length > 0 ? { id: "git", label: "Git", enabled: true, submenu } : null;
}

/** What only the window knows: the stream's changes now, and how the
 *  items run. `App` sets it (`setGitMenuHost`). */
export interface GitMenuHost extends GitMenuEnv {
  /** The stream's changed paths (`vcs.status`); rejects without a VCS. */
  status(): Promise<GitStatusEntry[]>;
}

let host: GitMenuHost | null = null;

/** The window says how Git ▸ runs (`App`); null when it closes. */
export function setGitMenuHost(next: GitMenuHost | null): void {
  host = next;
}

/** The Git ▸ item for `ref` in this window, as its menu opens; null for
 *  a ref that isn't a working-copy path, outside a window, or in a stream
 *  with no VCS (its status can't be read). */
export async function gitItemFor(ref: string | null, extra: MenuItem[] = []): Promise<MenuItem | null> {
  const target = ref ? gitTargetOf(ref) : null;
  const h = host;
  if (!target || !h) return null;
  const status = await h.status().catch(() => null);
  if (!status) return null;
  return gitMenu(target, status, h, extra);
}

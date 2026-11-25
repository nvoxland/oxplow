import { expect, test } from "bun:test";

import { gitMenu, gitTargetOf, type GitMenuEnv, type GitStatusEntry } from "./gitMenu.js";
import type { MenuItem } from "./menu.js";

// A file's (or folder's) Git ▸: only that path's actions — never the
// repository's (push, pull, branches) — and only those that apply to it now.

function env(): { env: GitMenuEnv; did: string[] } {
  const did: string[] = [];
  return {
    did,
    env: {
      openRef: (ref) => did.push(`open:${ref}`),
      compareWith: (path) => did.push(`compare:${path}`),
      rollback: (paths, checked) => did.push(`rollback:${paths.join(",")}|${checked.join(",")}`),
      ignore: (path) => did.push(`ignore:${path}`),
    },
  };
}

const labels = (item: MenuItem | null) => item?.submenu?.map((i) => i.label) ?? null;
const pick = (item: MenuItem | null, label: string) => item!.submenu!.find((i) => i.label === label)!;
const status = (...entries: [string, GitStatusEntry["status"]][]): GitStatusEntry[] => entries.map(([path, s]) => ({ path, status: s }));

test("an unchanged file: blame, history, compare", () => {
  const { env: e, did } = env();
  const menu = gitMenu({ kind: "file", path: "src/a.ts" }, status(["src/b.ts", "modified"]), e);
  expect(menu?.label).toBe("Git");
  expect(labels(menu)).toEqual(["Annotate with Git Blame", "Show History", "Compare with…"]);
  pick(menu, "Annotate with Git Blame").run!();
  pick(menu, "Show History").run!();
  pick(menu, "Compare with…").run!();
  expect(did).toEqual([
    "open:oxplow:oxplow/file?path=src/a.ts#oxplow.blame=1",
    "open:oxplow:oxplow/page?page=git-history&path=src/a.ts",
    "compare:src/a.ts",
  ]);
});

test("a changed file adds its diff and a rollback with it checked", () => {
  const { env: e, did } = env();
  const changes = status(["src/a.ts", "modified"], ["src/b.ts", "added"]);
  const menu = gitMenu({ kind: "file", path: "src/a.ts" }, changes, e);
  expect(labels(menu)).toEqual(["Annotate with Git Blame", "Show History", "Show Diff", "Compare with…", "Rollback…"]);
  pick(menu, "Show Diff").run!();
  pick(menu, "Rollback…").run!();
  expect(did[0]).toStartWith("open:oxplow:oxplow/page?");
  expect(did[0]).toContain("page=diff");
  expect(did[0]).toContain("left=git:HEAD");
  expect(did[1]).toBe("rollback:src/a.ts,src/b.ts|src/a.ts");
});

test("an untracked file: only Add to .gitignore, and Rollback (which deletes it)", () => {
  const { env: e, did } = env();
  const menu = gitMenu({ kind: "file", path: "notes.txt" }, status(["notes.txt", "untracked"]), e);
  expect(labels(menu)).toEqual(["Rollback…", "Add to .gitignore"]);
  pick(menu, "Add to .gitignore").run!();
  expect(did).toEqual(["ignore:notes.txt"]);
});

test("a folder: history, and its changes' diff and rollback, those under it checked", () => {
  const { env: e, did } = env();
  const changes = status(["src/a.ts", "modified"], ["src/sub/c.ts", "deleted"], ["docs/x.md", "modified"]);
  const menu = gitMenu({ kind: "dir", path: "src" }, changes, e);
  expect(labels(menu)).toEqual(["Show History", "Show Diff", "Rollback…"]);
  pick(menu, "Show Diff").run!();
  pick(menu, "Rollback…").run!();
  expect(did).toEqual(["open:oxplow:oxplow/page?page=uncommitted-changes", "rollback:src/a.ts,src/sub/c.ts,docs/x.md|src/a.ts,src/sub/c.ts"]);
  expect(labels(gitMenu({ kind: "dir", path: "lib" }, changes, e))).toEqual(["Show History"]);
});

test("an untracked folder can be ignored", () => {
  const menu = gitMenu({ kind: "dir", path: "out" }, status(["out/", "untracked"]), env().env);
  expect(labels(menu)).toEqual(["Rollback…", "Add to .gitignore"]);
});

test("an extension's Git commands join after oxplow's", () => {
  const extra: MenuItem[] = [{ id: "x", label: "Blame in Tool", enabled: true }];
  expect(labels(gitMenu({ kind: "file", path: "a.ts" }, [], env().env, extra))!.at(-1)).toBe("Blame in Tool");
});

test("only a working-copy file or folder on the local filesystem has a Git ▸", () => {
  expect(gitTargetOf("oxplow:oxplow/file?path=src%2Fa.ts")).toEqual({ kind: "file", path: "src/a.ts" });
  expect(gitTargetOf("oxplow:oxplow/dir?path=src")).toEqual({ kind: "dir", path: "src" });
  expect(gitTargetOf("oxplow:oxplow/file?path=a.ts&rev=git%3AHEAD")).toBeNull();
  expect(gitTargetOf("oxplow:oxplow/file?path=a.ts&filesystem=oxplow%3As3")).toBeNull();
  expect(gitTargetOf("oxplow:oxplow/dir?path=")).toBeNull();
  expect(gitTargetOf("https://x.io")).toBeNull();
});

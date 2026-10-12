import { afterEach, beforeEach, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// oxplow-bundled's history graph (the `vcs.history.graph` replacement): its
// bundle's own script, run against its own page with a stand-in for the
// client library's connection — the lanes, the filters, the reveal and
// opening a commit, as the frame shows them.

const dir = join(import.meta.dir, "../../../../extensions/oxplow-bundled/components/history-graph");
const page = readFileSync(join(dir, "index.html"), "utf8").replace(/<script[\s\S]*?<\/script>/g, "");
const script = readFileSync(join(dir, "app.js"), "utf8");

const COLUMNS = ["sha", "author", "email", "committed_at", "subject", "parents", "branches", "tags", "current_branch"];
type Commit = { sha: string; parents?: string[]; author?: string; subject?: string; at?: string; branches?: string[]; tags?: string[] };
const run = (commits: Commit[], reveal: string | null = null) => ({
  lens: { id: "oxplow-bundled/history-graph" },
  params: { reveal_sha: reveal },
  result: {
    columns: COLUMNS,
    rows: commits.map((c, i) => [
      c.sha,
      c.author ?? "ada",
      "ada@example.com",
      c.at ?? new Date(Date.UTC(2026, 0, 10 - i)).toISOString(),
      c.subject ?? `subject ${c.sha}`,
      JSON.stringify(c.parents ?? []),
      JSON.stringify(c.branches ?? []),
      JSON.stringify(c.tags ?? []),
      "main",
    ]),
    truncated: false,
  },
});

let updates: Array<(r: unknown) => void> = [];
let opened: Array<[string, unknown]> = [];

/** Load the page and start its script on `first`. */
async function start(first: ReturnType<typeof run>) {
  document.body.innerHTML = page;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (globalThis as any).oxplow = {
    connect: async () => ({
      run: first,
      applyTheme() {},
      onUpdate(fn: (r: unknown) => void) {
        updates.push(fn);
        return () => {};
      },
      navigate: async (ref: string, opts: unknown) => {
        opened.push([ref, opts]);
        return null;
      },
    }),
  };
  new Function(script)();
  await new Promise((r) => setTimeout(r, 0));
}

const rows = () => [...document.querySelectorAll<HTMLElement>("#rows .row")];
/** The column each row's dot is in. */
const columns = () => rows().map((r) => (Number(r.querySelector("circle")!.getAttribute("cx")) - 8 - 7) / 14);

beforeEach(() => {
  updates = [];
  opened = [];
});
afterEach(() => {
  document.body.innerHTML = "";
});

test("a straight history stays in one lane", async () => {
  await start(run([{ sha: "c", parents: ["b"] }, { sha: "b", parents: ["a"] }, { sha: "a" }]));
  expect(rows().map((r) => r.dataset.sha)).toEqual(["c", "b", "a"]);
  expect(columns()).toEqual([0, 0, 0]);
  expect(document.getElementById("count")!.textContent).toBe("3 / 3");
});

test("a merge opens a lane for its second parent that folds back", async () => {
  await start(
    run([
      { sha: "m", parents: ["a", "b"] },
      { sha: "a", parents: ["z"] },
      { sha: "b", parents: ["z"] },
      { sha: "z" },
    ]),
  );
  expect(columns()).toEqual([0, 0, 1, 0]);
  // The merge row draws an edge from its own lane into the new one.
  expect(rows()[0]!.querySelectorAll("line.lane-1").length).toBeGreaterThan(0);
});

test("children come before their parents whatever their times", async () => {
  // The parent's clock ran ahead of its child's.
  await start(run([{ sha: "p", at: "2026-01-10T00:00:00Z" }, { sha: "c", parents: ["p"], at: "2026-01-09T00:00:00Z" }]));
  expect(rows().map((r) => r.dataset.sha)).toEqual(["c", "p"]);
});

test("a click opens the commit; a Cmd-click opens it in a new tab", async () => {
  await start(run([{ sha: "c", parents: ["a"] }, { sha: "a" }]));
  rows()[0]!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  rows()[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, metaKey: true }));
  expect(opened).toEqual([
    ["commit:c", { newTab: false }],
    ["commit:a", { newTab: true }],
  ]);
});

test("the filters dim or hide rows without asking oxplow again", async () => {
  await start(
    run([
      { sha: "f2", parents: ["f1"], branches: ["feature"], subject: "tweak" },
      { sha: "f1", parents: ["a"], subject: "feature work", author: "bo" },
      { sha: "m", parents: ["a"], branches: ["main"], subject: "fix bug" },
      { sha: "a", subject: "start" },
    ]),
  );
  const query = document.getElementById("query") as HTMLInputElement;
  query.value = "bug";
  query.dispatchEvent(new Event("input"));
  expect(rows().filter((r) => !r.classList.contains("dimmed")).map((r) => r.dataset.sha)).toEqual(["m"]);
  expect(document.getElementById("count")!.textContent).toBe("1 / 4");

  query.value = "";
  const branch = document.getElementById("branch") as HTMLSelectElement;
  expect([...branch.options].map((o) => o.value)).toEqual(["", "feature", "main"]);
  branch.value = "feature";
  branch.dispatchEvent(new Event("input"));
  expect(rows().map((r) => r.dataset.sha)).toEqual(["f2", "f1", "a"]);

  const author = document.getElementById("author") as HTMLSelectElement;
  author.value = "bo";
  author.dispatchEvent(new Event("input"));
  expect(rows().filter((r) => !r.classList.contains("dimmed")).map((r) => r.dataset.sha)).toEqual(["f1"]);
});

test("the revealed commit is marked, and a re-run keeps the filters", async () => {
  await start(run([{ sha: "c", parents: ["a"] }, { sha: "a" }], "a"));
  expect(rows().filter((r) => r.classList.contains("revealed")).map((r) => r.dataset.sha)).toEqual(["a"]);
  const query = document.getElementById("query") as HTMLInputElement;
  query.value = "subject n";
  query.dispatchEvent(new Event("input"));
  updates[0]!(run([{ sha: "n", parents: ["c"] }, { sha: "c", parents: ["a"] }, { sha: "a" }]));
  expect(rows().map((r) => r.dataset.sha)).toEqual(["n", "c", "a"]);
  expect(rows().filter((r) => !r.classList.contains("dimmed")).map((r) => r.dataset.sha)).toEqual(["n"]);
});

test("no commits says so", async () => {
  await start(run([]));
  expect(document.getElementById("empty")!.hidden).toBe(false);
});

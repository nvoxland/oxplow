import { expect, test } from "bun:test";

import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { launcherCounts, launcherDirectory, launcherPages } from "./extensionLauncher.js";

const page = (over: Record<string, unknown>) => ({
  icon: null,
  category: null,
  keywords: null,
  params: [],
  count: null,
  body: { kind: "lens", lens: "x/y" },
  ...over,
});
const ext = (over: Partial<Extension>): Extension =>
  ({ name: "x", enabled: true, lenses: [], pages: [], ...over }) as unknown as Extension;

test("the launcher lists every declared page with a category, by its tab id", () => {
  const out = launcherPages([
    ext({
      name: "oxplow-foundation",
      pages: [
        page({ id: "tasks", extension: "oxplow-foundation", pageRef: "oxplow:oxplow/page?page=tasks", title: "Tasks", category: "Work", keywords: "dashboard" }),
        page({ id: "diff", extension: "oxplow-foundation", pageRef: "oxplow:oxplow/page?page=diff", title: "Diff" }),
      ],
    } as never),
    ext({ name: "gh", pages: [page({ id: "open-prs", extension: "gh", pageRef: "oxplow:oxplow/page?page=gh/open-prs", title: "Open PRs", category: "Work" })] } as never),
    ext({ name: "off", enabled: false, pages: [page({ id: "x", extension: "off", pageRef: "oxplow:oxplow/page?page=off/x", title: "X", category: "Work" })] } as never),
  ]);
  const rows = out.map((p) => [p.label, p.category, p.ref.id]);
  expect(rows).toContainEqual(["Tasks", "Work", "oxplow:oxplow/page?page=tasks"]);
  expect(rows).toContainEqual(["Open PRs", "Work", "oxplow:oxplow/page?page=gh/open-prs"]);
  // No category: opened from elsewhere, not the launcher. A disabled extension's pages aren't there.
  expect(rows.map((r) => r[0])).not.toContain("Diff");
  expect(rows.map((r) => r[0])).not.toContain("X");
  expect(out.find((p) => p.label === "Tasks")?.keywords).toContain("dashboard");
});

test("a page's count lens is its badge, and zero shows none", () => {
  const exts = [
    ext({
      name: "oxplow-foundation",
      pages: [page({ id: "backlog", extension: "oxplow-foundation", pageRef: "oxplow:oxplow/page?page=backlog", title: "Backlog", category: "Work", count: "oxplow-foundation/backlog-count" })],
    } as never),
  ];
  expect(launcherPages(exts, { "oxplow-foundation/backlog-count": 4 })[0]?.badge).toBe(4);
  expect(launcherPages(exts, { "oxplow-foundation/backlog-count": 0 })[0]?.badge).toBeUndefined();
});

test("until the extensions load, the launcher lists no pages rather than some of them", () => {
  expect(launcherDirectory(null, {})).toEqual([]);
  const loaded = launcherDirectory(
    [ext({ name: "oxplow-foundation", pages: [page({ id: "settings", extension: "oxplow-foundation", pageRef: "oxplow:oxplow/page?page=settings", title: "Settings", category: "System" })] } as never)],
    {},
  );
  expect(loaded.map((p) => p.label)).toEqual(["Settings"]);
});

test("a page whose lens has a view lists each other section by name, opening on it", () => {
  const settings = {
    id: "oxplow-foundation/settings",
    hidden: true,
    view: "area",
    params: [{ name: "area", default: "overview", options: [{ value: "overview", label: "Every Setting" }, { value: "agents", label: "Agents" }, { value: "ai", label: "AI" }] }],
  };
  const out = launcherPages([
    ext({
      name: "oxplow-foundation",
      lenses: [settings],
      pages: [page({ id: "settings", extension: "oxplow-foundation", pageRef: "oxplow:oxplow/page?page=settings", title: "Settings", category: "System", keywords: "config", body: { kind: "lens", lens: "oxplow-foundation/settings" } })],
    } as never),
  ]);
  const rows = out.map((p) => [p.label, p.category, p.ref.id]);
  expect(rows).toEqual([
    ["Settings", "System", "oxplow:oxplow/page?page=settings"],
    ["Settings: Agents", "System", "oxplow:oxplow/page?page=settings#area=agents"],
    ["Settings: AI", "System", "oxplow:oxplow/page?page=settings#area=ai"],
  ]);
  expect(out[1]?.keywords).toContain("config");
  expect(out[1]?.keywords).toContain("Agents");
});

// Page files (`pages/*.yaml`): what the launcher lists once lenses go.
const file = (extension: string, slug: string, over: Record<string, unknown> = {}) => ({
  id: `${extension}/${slug}`,
  extension,
  slug,
  title: slug,
  icon: null,
  category: null,
  keywords: null,
  hidden: false,
  params: [],
  badge: null,
  view: { id: "v", kind: "markdown", text: "" },
  ...over,
});

test("the launcher lists every page file with a category that isn't hidden, by its route", () => {
  const out = launcherPages([
    ext({
      name: "oxplow-foundation",
      namespace: "oxplow",
      pageFiles: [file("oxplow-foundation", "tasks", { title: "Tasks", category: "Work", keywords: "dashboard" }), file("oxplow-foundation", "diff"), file("oxplow-foundation", "secret", { category: "Work", hidden: true })],
      // The same page's `pages:` entry is listed once, as its file.
      pages: [page({ id: "tasks", extension: "oxplow-foundation", pageRef: "oxplow:oxplow/page?page=tasks", title: "Tasks (lens)", category: "Work" })],
    } as never),
    ext({ name: "gh", namespace: "gh", pageFiles: [file("gh", "open-prs", { title: "Open PRs", category: "Work" })] } as never),
  ]);
  const rows = out.map((p) => [p.label, p.category, p.ref.id]);
  expect(rows).toContainEqual(["Tasks", "Work", "oxplow:oxplow/page?page=tasks"]);
  expect(rows).toContainEqual(["Open PRs", "Work", "oxplow:oxplow/page?page=gh/open-prs"]);
  expect(rows.map((r) => r[0])).not.toContain("Tasks (lens)");
  expect(rows.map((r) => r[0])).not.toContain("diff");
  expect(rows.map((r) => r[0])).not.toContain("secret");
  expect(out.find((p) => p.label === "Tasks")?.keywords).toContain("dashboard");
});

test("a page file's badge count is its badge, and zero shows none", () => {
  const exts = [ext({ name: "oxplow-foundation", namespace: "oxplow", pageFiles: [file("oxplow-foundation", "backlog", { title: "Backlog", category: "Work", badge: { loader: "n", column: null } })] } as never)];
  expect(launcherPages(exts, { "oxplow-foundation/backlog": 4 })[0]?.badge).toBe(4);
  expect(launcherPages(exts, { "oxplow-foundation/backlog": 0 })[0]?.badge).toBeUndefined();
});

test("a page file whose view is a switch lists each other section by name, opening on it", () => {
  const settings = file("oxplow-foundation", "settings", {
    title: "Settings",
    category: "System",
    keywords: "config",
    params: [{ name: "area", default: "overview", role: "filter" }],
    view: {
      id: "area",
      kind: "switch",
      param: "area",
      children: [
        { id: "overview", title: "Every Setting", kind: "markdown", text: "" },
        { id: "agents", title: "Agents", kind: "markdown", text: "" },
        { id: "ai", title: null, kind: "markdown", text: "" },
      ],
    },
  });
  const out = launcherPages([ext({ name: "oxplow-foundation", namespace: "oxplow", pageFiles: [settings] } as never)]);
  expect(out.map((p) => [p.label, p.ref.id])).toEqual([
    ["Settings", "oxplow:oxplow/page?page=settings"],
    ["Settings: Agents", "oxplow:oxplow/page?page=settings#area=agents"],
    ["Settings: ai", "oxplow:oxplow/page?page=settings#area=ai"],
  ]);
  expect(out[1]?.keywords).toContain("config");
});

test("the counts are each badge page's one-shot run's badge, for the stream", async () => {
  const asked: unknown[][] = [];
  const exts = [
    ext({
      name: "oxplow-foundation",
      namespace: "oxplow",
      pageFiles: [
        file("oxplow-foundation", "backlog", { category: "Work", badge: { loader: "n", column: null } }),
        file("oxplow-foundation", "broken", { category: "Work", badge: { loader: "n", column: null } }),
        file("oxplow-foundation", "tasks", { category: "Work" }),
      ],
    } as never),
  ];
  const counts = await launcherCounts(exts, "str1", async (page, streamId) => {
    asked.push([page, streamId]);
    if (page === "oxplow-foundation/broken") throw new Error("no");
    return { badge: page === "oxplow-foundation/backlog" ? 3 : null };
  });
  expect(asked).toEqual([
    ["oxplow-foundation/backlog", "str1"],
    ["oxplow-foundation/broken", "str1"],
  ]);
  expect(counts).toEqual({ "oxplow-foundation/backlog": 3, "oxplow-foundation/broken": 0 });
});

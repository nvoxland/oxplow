import { expect, test } from "bun:test";

import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { launcherCounts, launcherDirectory, launcherPages } from "./extensionLauncher.js";

const page = (over: Record<string, unknown>) => ({
  icon: null,
  category: null,
  keywords: null,
  params: [],
  count: null,
  body: { kind: "page", page: "x/y" },
  ...over,
});
const ext = (over: Partial<Extension>): Extension =>
  ({ name: "x", enabled: true, ...over }) as unknown as Extension;

// Page files (`pages/*.yaml`): what the launcher lists.
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
    } as never),
    ext({ name: "gh", namespace: "gh", pageFiles: [file("gh", "open-prs", { title: "Open PRs", category: "Work" })] } as never),
  ]);
  const rows = out.map((p) => [p.label, p.category, p.ref.id]);
  expect(rows).toContainEqual(["Tasks", "Work", "oxplow:oxplow/page?page=tasks"]);
  expect(rows).toContainEqual(["Open PRs", "Work", "oxplow:oxplow/page?page=gh/open-prs"]);
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

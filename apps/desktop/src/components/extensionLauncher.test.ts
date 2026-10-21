import { expect, test } from "bun:test";

import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { launcherDirectory, launcherPages } from "./extensionLauncher.js";
import { settingsRef } from "../tabs/pageRefs.js";

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
  const rows = out.map((p) => [p.label, p.category, p.ref.id, p.ref.kind]);
  expect(rows).toContainEqual(["Tasks", "Work", "oxplow:oxplow/page?page=tasks", "page"]);
  expect(rows).toContainEqual(["Open PRs", "Work", "oxplow:oxplow/page?page=gh/open-prs", "page"]);
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
  const settingsArea = { id: "settings-agents", label: "Settings: Agents", ref: settingsRef("agents"), category: "System" as const };
  expect(launcherDirectory(null, {}, [settingsArea])).toEqual([]);
  const loaded = launcherDirectory(
    [ext({ name: "oxplow-foundation", pages: [page({ id: "settings", extension: "oxplow-foundation", pageRef: "oxplow:oxplow/page?page=settings", title: "Settings", category: "System" })] } as never)],
    {},
    [settingsArea],
  );
  // The declared page leads its category; the area entries follow it.
  expect(loaded.map((p) => p.label)).toEqual(["Settings", "Settings: Agents"]);
});

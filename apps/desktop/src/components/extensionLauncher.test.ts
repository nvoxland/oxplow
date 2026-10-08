import { expect, test } from "bun:test";

import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { launcherPages } from "./extensionLauncher.js";

const ext = (over: Partial<Extension>): Extension =>
  ({ name: "x", enabled: true, lenses: [], pages: [], ...over }) as unknown as Extension;

import { extPageRef, refFromTabId, pageKindOf } from "../tabs/pageRefs.js";

test("an extension's pages open as page:ext.<extension>.<page> and are in the launcher", () => {
  const ref = extPageRef("gh", "open-prs");
  expect(ref).toEqual({ id: "page:ext.gh.open-prs", kind: "ext-page", payload: { extension: "gh", page: "open-prs" } });
  expect(refFromTabId("page:ext.gh.open-prs")).toEqual(ref);
  expect(pageKindOf("page:ext.gh.open-prs")).toBe("ext-page");
  const out = launcherPages([
    ext({
      name: "gh",
      pages: [{ id: "open-prs", extension: "gh", pageRef: "page:ext.gh.open-prs", title: "Open PRs", icon: null, category: "Work", lens: "gh/open" }],
    } as never),
  ]);
  expect(out.map((p) => [p.label, p.category, p.ref.id])).toContainEqual(["Open PRs", "Work", "page:ext.gh.open-prs"]);
});

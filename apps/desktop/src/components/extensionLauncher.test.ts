import { expect, test } from "bun:test";

import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { launcherDirectory } from "./extensionLauncher.js";

const ext = (over: Partial<Extension>): Extension =>
  ({ name: "x", enabled: true, lenses: [], launcher: [], pages: [], ...over }) as unknown as Extension;

test("an extension's launcher entries: a ref is a page, a command or prompt an action", () => {
  const out = launcherDirectory([
    ext({
      launcher: [
        { label: "Settings", category: "System", target: { kind: "ref", ref: "commit:abc123" } },
        { label: "New Bug", category: "Work", target: { kind: "command", command: "oxplow.work_item.create", input: { title: "Bug" } } },
        { label: "Why slow", category: "Code", target: { kind: "prompt", prompt: "Why is the build slow?" } },
      ],
    }),
    ext({
      name: "off",
      enabled: false,
      launcher: [{ label: "Hidden", category: "Work", target: { kind: "prompt", prompt: "no" } }],
    }),
  ]);
  expect(out.pages.map((p) => [p.label, p.category, p.ref.id])).toEqual([["Settings", "System", "commit:abc123"]]);
  expect(out.actions).toEqual([
    { id: "x:New Bug", extension: "x", label: "New Bug", category: "Work", target: { kind: "command", command: "oxplow.work_item.create", input: { title: "Bug" } } },
    { id: "x:Why slow", extension: "x", label: "Why slow", category: "Code", target: { kind: "prompt", prompt: "Why is the build slow?" } },
  ]);
});

import { extPageRef, refFromTabId, pageKindOf } from "../tabs/pageRefs.js";

test("an extension's pages open as page:ext.<extension>.<page> and are in the launcher", () => {
  const ref = extPageRef("gh", "open-prs");
  expect(ref).toEqual({ id: "page:ext.gh.open-prs", kind: "ext-page", payload: { extension: "gh", page: "open-prs" } });
  expect(refFromTabId("page:ext.gh.open-prs")).toEqual(ref);
  expect(pageKindOf("page:ext.gh.open-prs")).toBe("ext-page");
  const out = launcherDirectory([
    ext({
      name: "gh",
      pages: [{ id: "open-prs", extension: "gh", pageRef: "page:ext.gh.open-prs", title: "Open PRs", icon: null, category: "Work", lens: "gh/open" }],
    } as never),
  ]);
  expect(out.pages.map((p) => [p.label, p.category, p.ref.id])).toContainEqual(["Open PRs", "Work", "page:ext.gh.open-prs"]);
});

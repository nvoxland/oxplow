import { expect, test } from "bun:test";

import type { MenuItem } from "./menu.js";
import { localTarget, openInMenu, type OpenInEnv } from "./openIn.js";

// Open In ▸ is one submenu for any path or URL the window recognizes:
// the OS file manager, the default app, a terminal there, the browser,
// oxplow's web tab — those that apply to the target and to this window.

function env(over: Partial<OpenInEnv> = {}): { env: OpenInEnv; did: string[] } {
  const did: string[] = [];
  return {
    did,
    env: {
      platform: "mac",
      filesLocal: true,
      reveal: async (p) => void did.push(`reveal:${p}`),
      openPath: async (p) => void did.push(`open:${p}`),
      openBrowser: async (u) => void did.push(`browser:${u}`),
      openInApp: (u) => did.push(`app:${u}`),
      inAppAllowed: () => true,
      openTerminalAt: (d) => did.push(`terminal:${d}`),
      ...over,
    },
  };
}

const labels = (item: MenuItem | null) => item?.submenu?.map((i) => i.label) ?? null;
const pick = (item: MenuItem | null, label: string) => item!.submenu!.find((i) => i.label === label)!;

test("a local file: the file manager, its default app, a terminal in its folder", async () => {
  const { env: e, did } = env();
  const menu = openInMenu({ kind: "file", path: "/w/src/a.ts" }, e);
  expect(menu?.label).toBe("Open In");
  expect(labels(menu)).toEqual(["Reveal in Finder", "Default App", "Terminal"]);
  pick(menu, "Reveal in Finder").run!();
  pick(menu, "Default App").run!();
  pick(menu, "Terminal").run!();
  await Promise.resolve();
  expect(did).toEqual(["reveal:/w/src/a.ts", "open:/w/src/a.ts", "terminal:/w/src"]);
});

test("a local directory: the file manager and a terminal there", () => {
  const { env: e, did } = env({ platform: "windows" });
  const menu = openInMenu({ kind: "dir", path: "/w/src" }, e);
  expect(labels(menu)).toEqual(["Show in Explorer", "Terminal"]);
  pick(menu, "Terminal").run!();
  expect(did).toEqual(["terminal:/w/src"]);
});

test("files on another machine get only the terminal; with no terminal, no menu", () => {
  expect(labels(openInMenu({ kind: "file", path: "/w/a.ts" }, env({ filesLocal: false }).env))).toEqual(["Terminal"]);
  expect(openInMenu({ kind: "file", path: "/w/a.ts" }, env({ filesLocal: false, openTerminalAt: null }).env)).toBeNull();
});

test("a web link: the browser, and oxplow's web tab when the allowlist takes it", () => {
  const { env: e, did } = env();
  const menu = openInMenu({ kind: "url", url: "https://example.com/x" }, e);
  expect(labels(menu)).toEqual(["Browser", "Oxplow"]);
  pick(menu, "Oxplow").run!();
  expect(did).toEqual(["app:https://example.com/x"]);
  expect(labels(openInMenu({ kind: "url", url: "https://example.com" }, env({ inAppAllowed: () => false }).env))).toEqual(["Browser"]);
});

test("an extension's items join after oxplow's", () => {
  const extra: MenuItem[] = [{ id: "x", label: "Jupyter", enabled: true, run: () => {} }];
  expect(labels(openInMenu({ kind: "file", path: "/w/n.ipynb" }, env().env, extra))).toEqual([
    "Reveal in Finder",
    "Default App",
    "Terminal",
    "Jupyter",
  ]);
  expect(labels(openInMenu({ kind: "file", path: "/w/n.ipynb" }, env({ filesLocal: false, openTerminalAt: null }).env, extra))).toEqual(["Jupyter"]);
});

test("a working-copy file URI on the local filesystem is a local path; another revision or filesystem is not", () => {
  expect(localTarget("oxplow:oxplow/file?path=src%2Fa.ts", "/w")).toEqual({ kind: "file", path: "/w/src/a.ts" });
  expect(localTarget("oxplow:oxplow/dir?path=src", "/w/")).toEqual({ kind: "dir", path: "/w/src" });
  expect(localTarget("oxplow:oxplow/dir?path=", "/w")).toEqual({ kind: "dir", path: "/w" });
  expect(localTarget("oxplow:oxplow/file?path=a.ts&rev=working", "/w")).toEqual({ kind: "file", path: "/w/a.ts" });
  expect(localTarget("oxplow:oxplow/file?path=a.ts&rev=git%3AHEAD", "/w")).toBeNull();
  expect(localTarget("oxplow:oxplow/file?path=a.ts&filesystem=oxplow%3As3", "/w")).toBeNull();
  expect(localTarget("oxplow:oxplow/task?provider=p&id=1", "/w")).toBeNull();
  expect(localTarget("https://example.com", "/w")).toEqual({ kind: "url", url: "https://example.com" });
  expect(localTarget("oxplow:oxplow/file?path=a.ts", null)).toBeNull();
});

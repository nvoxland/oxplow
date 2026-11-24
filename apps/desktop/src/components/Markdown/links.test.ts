import { afterEach, expect, test } from "bun:test";

import { setOpenInHost } from "../../openIn.js";
import { linkMenu } from "./links.js";

// A link's right-click menu: open it here, Open In ▸ somewhere else (the
// browser, oxplow's web tab, a terminal), copy what it names.

afterEach(() => setOpenInHost(null));

const host = (did: string[]) =>
  setOpenInHost({ worktree: "/w", openInApp: (u) => did.push(`app:${u}`), openTerminalAt: (d) => did.push(`terminal:${d}`) });

test("a web link: Open In ▸ Browser and Oxplow, then Copy link", () => {
  const did: string[] = [];
  host(did);
  const items = linkMenu("https://example.com/a", () => {});
  expect(items.map((i) => i.label)).toEqual(["Open In", "Copy link"]);
  expect(items[0]!.submenu!.map((i) => i.label)).toEqual(["Browser", "Oxplow"]);
  items[0]!.submenu![1]!.run!();
  expect(did).toEqual(["app:https://example.com/a"]);
});

test("a file link: Open, Open in new tab, Open In ▸ a terminal in its folder, Copy path", () => {
  const did: string[] = [];
  host(did);
  const items = linkMenu("oxplow:oxplow/file?path=src%2Flib%2Fa.ts", () => {});
  expect(items.map((i) => i.label)).toEqual(["Open", "Open in new tab", "Open In", "Copy path"]);
  const terminal = items[2]!.submenu!.find((i) => i.label === "Terminal")!;
  terminal.run!();
  expect(did).toEqual(["terminal:/w/src/lib"]);
});

test("a file at another revision has no Open In", () => {
  host([]);
  expect(linkMenu("oxplow:oxplow/file?path=a.ts&rev=git%3AHEAD", () => {}).map((i) => i.label)).toEqual(["Open", "Open in new tab", "Copy path"]);
});

test("a directory at another revision, or a file on another filesystem, has no Open In", () => {
  host([]);
  for (const href of ["oxplow:oxplow/dir?path=src&rev=git%3AHEAD", "oxplow:oxplow/file?path=a%2Fb.ts&filesystem=oxplow%3Aext"]) {
    expect(linkMenu(href, () => {}).some((i) => i.id === "open-in")).toBe(false);
  }
});

test("outside a window (no host) a file link has no Open In", () => {
  expect(linkMenu("oxplow:oxplow/file?path=src%2Fa.ts", () => {}).map((i) => i.label)).toEqual(["Open", "Open in new tab", "Copy path"]);
});

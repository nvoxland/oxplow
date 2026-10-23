import { expect, test } from "bun:test";

import golden from "../../../../crates/oxplow-app/fixtures/built_in_pages.json";
import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { APP_RENDERERS, BUILT_IN_PAGES, declaredPage, isBuiltInPage } from "./declaredPages.js";

// Intent kept: the Rust table is mirrored whole; the TS side splits it in two
// (renderers needing window state, and the rest).
test("the built-in pages are the Rust table's", () => {
  expect([...APP_RENDERERS, ...BUILT_IN_PAGES].sort()).toEqual([...golden].sort());
});

test("a built-in page name is recognized, anything else isn't", () => {
  expect(isBuiltInPage("terminal")).toBe(true);
  expect(isBuiltInPage("problems")).toBe(false);
});

const ext = (name: string, enabled: boolean, pageRef: string, id: string) =>
  ({ name, enabled, pages: [{ id, pageRef, title: id }] }) as unknown as Extension;

test("a page resolves by its tab id among the enabled extensions", () => {
  const exts = [ext("oxplow-foundation", true, "oxplow:oxplow/page?page=tasks", "tasks"), ext("gh", true, "oxplow:oxplow/page?page=gh/open-prs", "open-prs")];
  expect(declaredPage(exts, { extension: null, page: "tasks", params: {} })?.id).toBe("tasks");
  expect(declaredPage(exts, { extension: "gh", page: "open-prs", params: {} })?.id).toBe("open-prs");
  expect(declaredPage(exts, { extension: "oxplow-foundation", page: "tasks", params: {} })).toBeNull();
});

test("a disabled extension's page doesn't resolve, and loading is undefined", () => {
  expect(declaredPage([ext("gh", false, "oxplow:oxplow/page?page=gh/open-prs", "open-prs")], { extension: "gh", page: "open-prs", params: {} })).toBeNull();
  expect(declaredPage(null, { extension: null, page: "tasks", params: {} })).toBeUndefined();
});

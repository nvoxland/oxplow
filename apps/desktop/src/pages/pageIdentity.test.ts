import { expect, test } from "bun:test";

import { diffRef, extPageRef, fileRef, lensRef, metricRef, pageTabRef } from "../tabs/pageRefs.js";
import { pageIdentity, withView } from "./pageIdentity.js";

// A page's identity is its ref without its view params: what it is, not
// how it is viewed. One mounted body per identity in a tab; visits count
// a page once.
test("a lens's and an extension page's params are their view; an extension page's ref says what it is", () => {
  expect(pageIdentity(lensRef("x/y", { range: "7d" }))).toBe("oxplow:oxplow/lens?lens=x/y");
  expect(pageIdentity(extPageRef("acme", "pulls", { state: "open" }))).toBe("oxplow:oxplow/page?page=acme/pulls");
  expect(pageIdentity(extPageRef("acme", "pr", { ref: "oxplow:oxplow/work_item?id=1&provider=gh", tab: "files" }))).toBe(extPageRef("acme", "pr", { ref: "oxplow:oxplow/work_item?id=1&provider=gh" }).id);
});

test("a built-in page's identity keeps every param but the view params it declares", () => {
  expect(pageIdentity(pageTabRef("tasks", { scope: "stream", stream: "str1", "f.state": "ready" }))).toBe("oxplow:oxplow/page?page=tasks");
  // A diff's params say which diff it is: two diffs are two pages.
  const a = diffRef({ path: "a.ts", leftVersion: { kind: "disk" }, rightVersion: { kind: "disk" } } as never);
  const b = diffRef({ path: "b.ts", leftVersion: { kind: "disk" }, rightVersion: { kind: "disk" } } as never);
  expect(pageIdentity(a)).not.toBe(pageIdentity(b));
  expect(pageIdentity(pageTabRef("custom-dashboard", { id: "7" }))).toBe("oxplow:oxplow/page?id=7&page=custom-dashboard");
  // A dashboard's range and branch are its view; its id is not.
  expect(pageIdentity(pageTabRef("custom-dashboard", { id: "7", range: "7d", branch: "main" }))).toBe("oxplow:oxplow/page?id=7&page=custom-dashboard");
  expect(pageIdentity(pageTabRef("metrics-recorded", { range: "30d", slice: "oxplow.language", q: "x", show: "all" }))).toBe("oxplow:oxplow/page?page=metrics-recorded");
});

test("the search pages' filters are view, so a change re-renders them", () => {
  expect(pageIdentity(pageTabRef("git-history", { q: "fix", author: "a", branch: "main" }))).toBe("oxplow:oxplow/page?page=git-history");
  expect(pageIdentity(pageTabRef("comments", { days: 7 }))).toBe("oxplow:oxplow/page?page=comments");
  expect(pageIdentity(pageTabRef("settings", { q: "model" }))).toBe("oxplow:oxplow/page?page=settings");
  expect(pageIdentity(pageTabRef("explore-data", { model: "v_effort", sql: "SELECT 1", viz: "bar", raw: true }))).toBe("oxplow:oxplow/page?page=explore-data");
  expect(pageIdentity(pageTabRef("new-session", { q: "billing" }))).toBe("oxplow:oxplow/page?page=new-session");
});

test("a mode or a pick is view: the Files filter and the open Alerts row", () => {
  expect(pageIdentity(pageTabRef("files", { filter: "branch" }))).toBe("oxplow:oxplow/page?page=files");
  expect(pageIdentity(pageTabRef("alerts", { open: "op-errors" }))).toBe("oxplow:oxplow/page?page=alerts");
  expect(pageIdentity(pageTabRef("board", { show: "backlog" }))).toBe("oxplow:oxplow/page?page=board");
});

test("an entity page is its ref, less the view its kind declares", () => {
  expect(pageIdentity(fileRef("src/a.ts"))).toBe(fileRef("src/a.ts").id);
  expect(pageIdentity(metricRef("oxplow.todos", { range: "30d", branch: "main", slice: "oxplow.language" }))).toBe("oxplow:oxplow/metric?metric=oxplow.todos");
  expect(withView(metricRef("oxplow.todos"), { range: "30d" }).id).toBe("oxplow:oxplow/metric?metric=oxplow.todos&range=30d");
  expect(() => withView(metricRef("oxplow.todos"), { sort: "age" })).toThrow("`sort` isn't a view param of oxplow:oxplow/metric?metric=oxplow.todos");
});

// A page changes only its view: writing a param it doesn't declare as view
// would make it another page (and mount it again), so it is refused, by name.
test("withView writes a page's view params and refuses one that would change what the page is", () => {
  expect(withView(pageTabRef("tasks"), { scope: "all" }).id).toBe("oxplow:oxplow/page?page=tasks&scope=all");
  expect(withView(lensRef("x/y"), { range: "7d" }).id).toBe(lensRef("x/y", { range: "7d" }).id);
  expect(() => withView(pageTabRef("tasks"), { sort: "age" })).toThrow(/`sort`.*oxplow:oxplow\/page\?page=tasks.*BUILT_IN_VIEW/);
  expect(() => withView(pageTabRef("custom-dashboard", { id: "7" }), { id: "7", sort: "age" })).toThrow(/^`sort` isn't/);
});

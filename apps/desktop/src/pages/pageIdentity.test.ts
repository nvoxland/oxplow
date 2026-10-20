import { expect, test } from "bun:test";

import { diffRef, extPageRef, fileRef, lensRef, pageTabRef } from "../tabs/pageRefs.js";
import { pageIdentity, withView } from "./pageIdentity.js";

// A page's identity is its ref without its view params: what it is, not
// how it is viewed. One mounted body per identity in a tab; visits count
// a page once.
test("a lens's and an extension page's params are their view; an extension page's ref says what it is", () => {
  expect(pageIdentity(lensRef("x/y", { range: "7d" }))).toBe("lens:x/y");
  expect(pageIdentity(extPageRef("acme", "pulls", { state: "open" }))).toBe("page:acme/pulls");
  expect(pageIdentity(extPageRef("acme", "pr", { ref: "work_item:gh:1", tab: "files" }))).toBe(extPageRef("acme", "pr", { ref: "work_item:gh:1" }).id);
});

test("a built-in page's identity keeps every param but the view params it declares", () => {
  expect(pageIdentity(pageTabRef("tasks", { scope: "stream", stream: "str1", "f.state": "ready" }))).toBe("page:tasks");
  // A diff's params say which diff it is: two diffs are two pages.
  const a = diffRef({ path: "a.ts", leftVersion: { kind: "disk" }, rightVersion: { kind: "disk" } } as never);
  const b = diffRef({ path: "b.ts", leftVersion: { kind: "disk" }, rightVersion: { kind: "disk" } } as never);
  expect(pageIdentity(a)).not.toBe(pageIdentity(b));
  expect(pageIdentity(pageTabRef("custom-dashboard", { id: "7" }))).toBe("page:custom-dashboard?id=7");
});

test("an entity page is its ref", () => {
  expect(pageIdentity(fileRef("src/a.ts"))).toBe(fileRef("src/a.ts").id);
});

// A page changes only its view: writing a param it doesn't declare as view
// would make it another page (and mount it again), so it is refused, by name.
test("withView writes a page's view params and refuses one that would change what the page is", () => {
  expect(withView(pageTabRef("tasks"), { scope: "all" }).id).toBe("page:tasks?scope=all");
  expect(withView(lensRef("x/y"), { range: "7d" }).id).toBe(lensRef("x/y", { range: "7d" }).id);
  expect(() => withView(pageTabRef("tasks"), { sort: "age" })).toThrow(/`sort`.*page:tasks.*BUILT_IN_VIEW/);
  expect(() => withView(pageTabRef("custom-dashboard", { id: "7" }), { id: "7", range: "7d" })).toThrow(/^`range` isn't/);
});

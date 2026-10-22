import { expect, test } from "bun:test";

import { commentUri, fileUri, resourceUri, streamUri, threadUri, wikiUri, workItemUri } from "../uri/uri.js";
import { fileRef, wikiPageRef, workItemTabRef } from "./pageRefs.js";
import { refTarget, refTargetNow } from "./refOpener.js";

// Every URI opens by one rule: as itself (its tab's id is the URI, drawn by
// the page that renders it, or by the fallback); a thread or a stream
// selects instead of opening a tab. The registry plays no part.
test("a URI opens as itself, a thread's or a stream's selects", () => {
  const item = workItemUri("oxplow", "tsk1");
  expect(refTargetNow(item)).toEqual({ tab: workItemTabRef(item) });
  expect(refTargetNow(wikiUri("home"))).toEqual({ tab: wikiPageRef("home") });
  expect(refTargetNow(fileUri("src/a.ts"))).toEqual({ tab: fileRef("src/a.ts") });
  expect(refTargetNow(threadUri("thr4"))).toEqual({ thread: "thr4" });
  expect(refTargetNow(streamUri("s1"))).toEqual({ stream: "s1" });
  const pr = resourceUri("acme/pr", { id: 12 });
  expect(refTargetNow(pr)).toEqual({ tab: { id: pr } });
});

test("an oxplow resource with no declared page opens as itself too", () => {
  // Intent moved: a resource no longer names a page that it opens with a `uri`.
  const comment = commentUri("cmt7");
  expect(refTargetNow(comment)).toEqual({ tab: { id: comment } });
});

test("before the registry is read, a URI opens as itself", () => {
  expect(refTargetNow(wikiUri("home"))).toEqual({ tab: wikiPageRef("home") });
  expect(refTargetNow(threadUri("thr4"))).toEqual({ thread: "thr4" });
});

test("refTarget is refTargetNow, awaited", async () => {
  expect(await refTarget(threadUri("thr4"))).toEqual({ thread: "thr4" });
});

test("a resource nobody declares opens as itself; only what is no URI opens nothing", async () => {
  // The fallback page shows a URI of an undeclared resource as a dead link.
  const gone = resourceUri("gone/kind", { id: 1 });
  expect(await refTarget(gone)).toEqual({ tab: { id: gone } });
  expect("unopenable" in (await refTarget("gone_kind:1"))).toBe(true);
});

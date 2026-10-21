import { expect, test } from "bun:test";

import { openRef, type RefHost } from "./openRef.js";
import type { RefTarget } from "./refOpener.js";
import { fileRef } from "./pageRefs.js";

/** A host that records what it was asked to do. */
function host(known: { threads?: string[]; streams?: string[] } = {}) {
  const log: string[] = [];
  const h: RefHost = {
    open: (tab, how) => log.push(`open ${tab.id}${how.newTab ? " new" : ""}`),
    showThread: (t) => (log.push(`thread ${t}`), known.threads?.includes(t) ?? false),
    showStream: (s) => (log.push(`stream ${s}`), known.streams?.includes(s) ?? false),
    failed: (m) => log.push(`failed ${m}`),
  };
  return { h, log };
}
const resolving = (target: RefTarget) => async () => target;

test("a tab target opens in the current tab, or a new one when asked", async () => {
  const { h, log } = host();
  const tab = fileRef("a.rs");
  await openRef("oxplow:oxplow/file?path=a.rs", {}, h, resolving({ tab }));
  await openRef("oxplow:oxplow/file?path=a.rs", { newTab: true }, h, resolving({ tab }));
  expect(log).toEqual([`open ${tab.id}`, `open ${tab.id} new`]);
});

test("a thread or a stream target selects it, and says so when there is none", async () => {
  const { h, log } = host({ threads: ["thr1"] });
  await openRef("oxplow:oxplow/thread?id=thr1", {}, h, resolving({ thread: "thr1" }));
  await openRef("oxplow:oxplow/thread?id=thr9", {}, h, resolving({ thread: "thr9" }));
  await openRef("oxplow:oxplow/stream?id=str1", {}, h, resolving({ stream: "str1" }));
  expect(log).toEqual([
    "thread thr1",
    "thread thr9",
    "failed no thread `thr9`",
    "stream str1",
    "failed no stream `str1`",
  ]);
});

test("an unopenable ref and a failed lookup are reported, never silent", async () => {
  const { h, log } = host();
  await openRef("oxplow:oxplow/event?id=e1", {}, h, resolving({ unopenable: "`oxplow:oxplow/event?id=e1` has no page" }));
  await openRef("x:1", {}, h, async () => {
    throw new Error("query failed");
  });
  expect(log).toEqual(["failed `oxplow:oxplow/event?id=e1` has no page", "failed query failed"]);
});

// A submitted form gives way to what it made: a stream or a thread is not a
// tab, so the page leaves (`replace`) once the stream or thread is shown.
test("a replace that shows a stream or a thread closes the page it came from", async () => {
  const { h, log } = host({ threads: ["thr1"], streams: ["str1"] });
  const closing = { ...h, closePage: () => log.push("close page") };
  await openRef("oxplow:oxplow/stream?id=str1", { replace: true }, closing, resolving({ stream: "str1" }));
  await openRef("oxplow:oxplow/thread?id=thr1", { replace: true }, closing, resolving({ thread: "thr1" }));
  await openRef("oxplow:oxplow/stream?id=str9", { replace: true }, closing, resolving({ stream: "str9" }));
  await openRef("oxplow:oxplow/stream?id=str1", {}, closing, resolving({ stream: "str1" }));
  expect(log).toEqual([
    "stream str1",
    "close page",
    "thread thr1",
    "close page",
    "stream str9",
    "failed no stream `str9`",
    "stream str1",
  ]);
});

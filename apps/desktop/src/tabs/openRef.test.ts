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
  await openRef("file:a.rs", {}, h, resolving({ tab }));
  await openRef("file:a.rs", { newTab: true }, h, resolving({ tab }));
  expect(log).toEqual([`open ${tab.id}`, `open ${tab.id} new`]);
});

test("a thread or a stream target selects it, and says so when there is none", async () => {
  const { h, log } = host({ threads: ["thr1"] });
  await openRef("thread:thr1", {}, h, resolving({ thread: "thr1" }));
  await openRef("thread:thr9", {}, h, resolving({ thread: "thr9" }));
  await openRef("stream:str1", {}, h, resolving({ stream: "str1" }));
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
  await openRef("event:e1", {}, h, resolving({ unopenable: "`event:e1` has no page" }));
  await openRef("x:1", {}, h, async () => {
    throw new Error("query failed");
  });
  expect(log).toEqual(["failed `event:e1` has no page", "failed query failed"]);
});

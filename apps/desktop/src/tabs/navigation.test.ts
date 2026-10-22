import { expect, test } from "bun:test";

import { planNavigation } from "./navigation.js";
import { wikiPageRef } from "./pageRefs.js";

const states = {
  str1: { selectedThreadId: "thr1", threads: [{ id: "thr1" }, { id: "thr2" }] },
  str2: { selectedThreadId: "thr5", threads: [{ id: "thr5" }] },
};

test("a ref opens its page, in the shown stream unless one is named", async () => {
  const plan = await planNavigation({ uri: "oxplow:oxplow/file?path=src/auth.rs" }, states, "str1");
  expect(plan.streamId).toBe("str1");
  expect(plan.threadId).toBe("thr1");
  expect(plan.page?.id).toBe("oxplow:oxplow/file?path=src/auth.rs");
  expect(plan.selectThread).toBe(false);
});

test("the pages a ref can name each resolve", async () => {
  for (const ref of ["oxplow:oxplow/work_item?id=tsk3&provider=oxplow", "oxplow:oxplow/effort?id=eff9", "oxplow:oxplow/lens?lens=oxplow/tasks", "oxplow:oxplow/wiki?slug=overview", "oxplow:oxplow/dir?path=src", "oxplow:oxplow/page?page=git-dashboard"]) {
    expect((await planNavigation({ uri: ref }, states, "str1")).page?.id).toBe(ref);
  }
});

test("a named stream is switched to, and its selected thread shows the page", async () => {
  const plan = await planNavigation({ uri: "oxplow:oxplow/wiki?slug=overview", stream: "oxplow:oxplow/stream?id=str2" }, states, "str1");
  expect(plan.streamId).toBe("str2");
  expect(plan.threadId).toBe("thr5");
  expect(plan.page?.id).toBe("oxplow:oxplow/wiki?slug=overview");
  expect((await planNavigation({ uri: "oxplow:oxplow/wiki?slug=overview", stream: "str2" }, states, "str1")).streamId).toBe("str2");
});

test("a bare stream or thread ref selects it and opens nothing", async () => {
  const stream = await planNavigation({ uri: "oxplow:oxplow/stream?id=str2" }, states, "str1");
  expect(stream).toMatchObject({ streamId: "str2", threadId: "thr5", selectThread: false });
  expect(stream.page).toBeNull();
  const thread = await planNavigation({ uri: "oxplow:oxplow/thread?id=thr2" }, states, "str2");
  expect(thread).toMatchObject({ streamId: "str1", threadId: "thr2", selectThread: true });
  expect(thread.page).toBeNull();
});

test("what can't be reached is refused", async () => {
  await expect(planNavigation({ uri: "nonsense" }, states, "str1")).rejects.toThrow();
  await expect(planNavigation({}, states, "str1")).rejects.toThrow(/uri/);
  await expect(planNavigation({ uri: "oxplow:oxplow/wiki?slug=x", stream: "oxplow:oxplow/stream?id=str9" }, states, "str1")).rejects.toThrow(/no stream/);
  await expect(planNavigation({ uri: "oxplow:oxplow/thread?id=thr9" }, states, "str1")).rejects.toThrow(/no thread/);
  await expect(planNavigation({ uri: "oxplow:oxplow/stream?id=str9" }, states, "str1")).rejects.toThrow(/no stream/);
  await expect(planNavigation({ uri: "oxplow:oxplow/wiki?slug=x" }, states, null)).rejects.toThrow(/no stream/);
});

// Where a ref opens is the opener's: a note opens its thread, whatever
// the opener reads to say so.
test("a ref goes where the opener says", async () => {
  const note = await planNavigation({ uri: "oxplow:oxplow/thread_note?id=not3" }, states, "str1", async () => ({ thread: "thr5" }));
  expect(note).toMatchObject({ streamId: "str2", threadId: "thr5", selectThread: true, page: null });
  const page = await planNavigation({ uri: "oxplow:oxplow/decision?id=4" }, states, "str1", async () => ({ tab: wikiPageRef("overview") }));
  expect(page.page?.id).toBe("oxplow:oxplow/wiki?slug=overview");
  await expect(planNavigation({ uri: "oxplow:oxplow/event?id=x" }, states, "str1", async () => ({ unopenable: "no page" }))).rejects.toThrow(/no page/);
});

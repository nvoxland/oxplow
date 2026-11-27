import { expect, test } from "bun:test";

import { knowledgePageRef } from "./pageRefs.js";
import { emptySession, open, serializeSession, type Session } from "./session.js";
import { applyEarly, createSessionSaver, sessionsFromStored } from "./useTabSessions.js";

// A device's tabs are the daemon's (`tab_session`): read at start, and each
// thread's saved as it changes, one step at a time, each save giving the
// revision it read, so a late save never overwrites a newer one. The live
// window owns its device's tabs: a refused save is sent again over where
// the row is.

const flush = async () => {
  for (let i = 0; i < 6; i += 1) await Promise.resolve();
};

test("the stored tabs read back by thread, and a bad thread drops alone", () => {
  const good = open(emptySession(), { ref: knowledgePageRef("oxwiki", "a") });
  const { sessions, revisions } = sessionsFromStored([
    { threadId: "thr1", sessionJson: serializeSession(good), revision: 3 },
    { threadId: "thr2", sessionJson: "{nope", revision: 1 },
  ]);
  expect(sessions).toEqual({ thr1: good });
  expect(revisions).toEqual({ thr1: 3 });
});

function backend() {
  const steps: string[] = [];
  const answers: Array<(a: { saved: boolean; revision: number | null }) => void> = [];
  let failNext = false;
  return {
    steps,
    answers,
    failOnce: () => (failNext = true),
    write: {
      save: (thread: string, json: string, revision: number | null) => {
        steps.push(`save ${thread} ${json} @${revision}`);
        if (failNext) {
          failNext = false;
          return Promise.reject(new Error("down"));
        }
        return new Promise<{ saved: boolean; revision: number | null }>((resolve) => answers.push(resolve));
      },
      forget: async (thread: string) => {
        steps.push(`forget ${thread}`);
      },
    },
  };
}

test("a thread's saves go one at a time, only the newest waiting, each with the revision the last gave", async () => {
  const b = backend();
  const saver = createSessionSaver(b.write, { thr1: 4 });
  saver.save("thr1", "a");
  saver.save("thr1", "b");
  saver.save("thr1", "c");
  expect(b.steps).toEqual(["save thr1 a @4"]);
  b.answers.shift()!({ saved: true, revision: 5 });
  await flush();
  expect(b.steps).toEqual(["save thr1 a @4", "save thr1 c @5"]);
});

test("a refused save is sent again over the revision the daemon has", async () => {
  const b = backend();
  const saver = createSessionSaver(b.write, { thr1: 4 });
  saver.save("thr1", "a");
  b.answers.shift()!({ saved: false, revision: 9 });
  await flush();
  expect(b.steps).toEqual(["save thr1 a @4", "save thr1 a @9"]);
});

test("a forget waits for the save in flight, and the next save starts a new row", async () => {
  const b = backend();
  const saver = createSessionSaver(b.write, { thr1: 4 });
  saver.save("thr1", "a");
  saver.forget("thr1");
  saver.save("thr1", "fresh");
  expect(b.steps).toEqual(["save thr1 a @4"]);
  b.answers.shift()!({ saved: true, revision: 5 });
  await flush();
  expect(b.steps).toEqual(["save thr1 a @4", "forget thr1", "save thr1 fresh @null"]);
});

test("a save that fails is kept, and sent again on retry", async () => {
  const b = backend();
  const saver = createSessionSaver(b.write, {});
  b.failOnce();
  saver.save("thr1", "a");
  await flush();
  expect(b.steps).toEqual(["save thr1 a @null"]);
  saver.retry();
  expect(b.steps).toEqual(["save thr1 a @null", "save thr1 a @null"]);
});

test("a thread is kept once every save before the question has been answered", async () => {
  const b = backend();
  const saver = createSessionSaver(b.write, {});
  let kept = 0;
  void saver.idle("thr2").then(() => (kept += 100));
  b.failOnce();
  saver.save("thr1", "a");
  void saver.idle("thr1").then(() => (kept += 1));
  await flush();
  expect(kept).toBe(100); // a thread with nothing to save is kept now; a failed save is not
  saver.retry();
  b.answers.shift()!({ saved: true, revision: 1 });
  await flush();
  expect(kept).toBe(101);
});

test("changes made before the stored tabs arrive are made over them", () => {
  const stored = open(emptySession(), { ref: knowledgePageRef("oxwiki", "kept") });
  const early: Array<[string, (s: Session) => Session]> = [
    ["thr1", (s) => open(s, { ref: knowledgePageRef("oxwiki", "new") })],
  ];
  const out = applyEarly({ thr1: stored }, early);
  expect(out.thr1!.tabs.map((t) => t.entries[0]!.ref.id)).toEqual([
    knowledgePageRef("oxwiki", "kept").id,
    knowledgePageRef("oxwiki", "new").id,
  ]);
});

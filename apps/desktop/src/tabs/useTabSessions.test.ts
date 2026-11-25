import { expect, test } from "bun:test";

import { knowledgePageRef } from "./pageRefs.js";
import { emptySession, open, serializeSession } from "./session.js";
import { createSessionSaver, sessionsFromStored } from "./useTabSessions.js";

// A device's tabs are the daemon's (`tab_session`): read at start, and each
// thread's saved as it changes, one save at a time, each giving the revision
// it read, so a late save never overwrites a newer one.

test("the stored tabs read back by thread, and a bad thread drops alone", () => {
  const good = open(emptySession(), { ref: knowledgePageRef("oxwiki", "a") });
  const { sessions, revisions } = sessionsFromStored([
    { threadId: "thr1", sessionJson: serializeSession(good), revision: 3 },
    { threadId: "thr2", sessionJson: "{nope", revision: 1 },
  ]);
  expect(sessions).toEqual({ thr1: good });
  expect(revisions).toEqual({ thr1: 3 });
});

test("a thread's saves go one at a time, the newest waiting, each with the revision the last one gave", async () => {
  const sent: Array<{ thread: string; json: string; revision: number | null }> = [];
  const answers: Array<(revision: number | null) => void> = [];
  const saver = createSessionSaver(
    (thread, json, revision) => {
      sent.push({ thread, json, revision });
      return new Promise((resolve) => answers.push(resolve));
    },
    { thr1: 4 },
    () => {},
  );
  saver.save("thr1", "a");
  saver.save("thr1", "b");
  saver.save("thr1", "c");
  expect(sent).toEqual([{ thread: "thr1", json: "a", revision: 4 }]);
  answers.shift()!(5);
  await Promise.resolve();
  await Promise.resolve();
  // Only the newest of the ones that waited is sent, with the new revision.
  expect(sent.at(-1)).toEqual({ thread: "thr1", json: "c", revision: 5 });
  expect(sent).toHaveLength(2);
});

test("a save the daemon refuses as stale is told, so the window takes the stored copy", async () => {
  const stale: string[] = [];
  const saver = createSessionSaver(async () => null, {}, (thread) => stale.push(thread));
  saver.save("thr2", "x");
  await Promise.resolve();
  await Promise.resolve();
  expect(stale).toEqual(["thr2"]);
});

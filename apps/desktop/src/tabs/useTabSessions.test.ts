import { afterEach, expect, test } from "bun:test";

import { wikiPageRef } from "./pageRefs.js";
import { emptySession, open } from "./session.js";
import { readPersistedSessions, SESSIONS_STORAGE_KEY, writePersistedSessions } from "./useTabSessions.js";

afterEach(() => window.localStorage.removeItem(SESSIONS_STORAGE_KEY));

test("every thread's session is stored under one key and read back", () => {
  const sessions = { thr1: open(emptySession(), { ref: wikiPageRef("a") }), thr2: emptySession() };
  writePersistedSessions(sessions);
  expect(readPersistedSessions()).toEqual(sessions);
});

test("stored text that isn't sessions reads as none, and a bad thread drops alone", () => {
  window.localStorage.setItem(SESSIONS_STORAGE_KEY, "{nope");
  expect(readPersistedSessions()).toEqual({});
  const good = open(emptySession(), { ref: wikiPageRef("a") });
  window.localStorage.setItem(SESSIONS_STORAGE_KEY, JSON.stringify({ thr1: good, thr2: { tabs: 3 } }));
  expect(Object.keys(readPersistedSessions())).toEqual(["thr1"]);
});

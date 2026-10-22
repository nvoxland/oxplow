import { expect, test } from "bun:test";

import { buildTabReport } from "./tabReport.js";
import { fileRef, wikiPageRef } from "./pageRefs.js";
import { back, emptySession, navigate, open } from "./session.js";

// A window reports every thread's tabs with their shown titles, the one
// selected in each and where each tab's history leads, the thread it
// shows, and its active page's detail.
test("a window's report lists every thread's tabs, selection and history", () => {
  let thr1 = open(emptySession(), { ref: wikiPageRef("home") });
  const slot = thr1.active!;
  thr1 = navigate(thr1, slot, { ref: wikiPageRef("start") });
  thr1 = navigate(thr1, slot, { ref: wikiPageRef("next") });
  thr1 = back(thr1, slot);
  thr1 = open(thr1, {
    ref: fileRef("src/a.rs"),
    from: { title: "Files in src", items: [{ ref: fileRef("src/a.rs"), label: "a.rs" }], index: 0 },
  });
  const report = buildTabReport({
    client: "w1",
    focused: true,
    visible: true,
    shownThread: "thr1",
    sessions: { thr1, thr2: emptySession() },
    titleOf: (tab) => (tab.id === "oxplow:oxplow/wiki?slug=start" ? "Start" : null),
    detail: { path: "src/a.rs" },
  });
  expect(report).toEqual({
    client: "w1",
    focused: true,
    visible: true,
    shown_thread: "thr1",
    threads: [
      {
        thread: "thr1",
        tabs: [
          { id: "oxplow:oxplow/wiki?slug=start", kind: "oxplow/wiki", title: "Start", back: ["oxplow:oxplow/wiki?slug=home"], forward: ["oxplow:oxplow/wiki?slug=next"], origin: null },
          { id: "oxplow:oxplow/file?path=src/a.rs", kind: "oxplow/file", title: "oxplow/file", back: [], forward: [], origin: "Files in src" },
        ],
        active: "oxplow:oxplow/file?path=src/a.rs",
      },
      { thread: "thr2", tabs: [], active: null },
    ],
    detail_json: '{"path":"src/a.rs"}',
  });
});

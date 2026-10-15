import { expect, test } from "bun:test";

import { buildTabReport } from "./tabReport.js";
import { fileRef, wikiPageRef } from "./pageRefs.js";

// A window reports every thread's tabs with their shown titles and the
// one selected in each, the thread it shows, and its active page's detail.
test("a window's report lists every thread's tabs and selection", () => {
  const report = buildTabReport({
    client: "w1",
    focused: true,
    visible: true,
    shownThread: "thr1",
    threadPageTabs: {
      thr1: [wikiPageRef("home"), fileRef("src/a.rs")],
      thr2: [wikiPageRef("notes")],
      thr3: [],
    },
    activeByThread: { thr1: "file:src/a.rs", thr2: "wiki:gone" },
    titleOf: (tab) => (tab.id === "wiki:home" ? "Home" : null),
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
          { id: "wiki:home", kind: "wiki", title: "Home" },
          { id: "file:src/a.rs", kind: "file", title: "file" },
        ],
        active: "file:src/a.rs",
      },
      // A selection that isn't one of its tabs (closed) is none.
      { thread: "thr2", tabs: [{ id: "wiki:notes", kind: "wiki", title: "wiki page" }], active: null },
      { thread: "thr3", tabs: [], active: null },
    ],
    detail_json: '{"path":"src/a.rs"}',
  });
});

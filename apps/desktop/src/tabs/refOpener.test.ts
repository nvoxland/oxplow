import { afterEach, expect, test } from "bun:test";

import { setRefKinds, type RefKind } from "../refKinds.js";
import { commentsRef, extPageRef, fileRef, wikiPageRef, workItemTabRef } from "./pageRefs.js";
import { refTarget, refTargetNow } from "./refOpener.js";

const core = (kind: string, page: string | null, resolve: string | null = null): RefKind => ({
  kind,
  extension: null,
  label: null,
  idPattern: ".*",
  wikilinks: [],
  resolve,
  page,
  icon: null,
});

const KINDS: RefKind[] = [
  core("work_item", "{ref}", "v_work_item"),
  core("wiki", "{ref}", "v_knowledge_page"),
  core("file", "{ref}"),
  core("thread", "{ref}", "v_thread"),
  core("comment", "page:comments", "v_comment"),
  core("thread_note", "{thread_ref}", "v_thread_note"),
  core("decision", "{effort_ref|thread_ref}", "v_decision"),
  core("event", null, "v_event"),
  {
    kind: "acme_pr",
    extension: "acme",
    label: "Pull request",
    idPattern: "^\\d+$",
    wikilinks: ["pr"],
    resolve: "v_acme_prs",
    page: "page:acme/pr?ref={ref}",
    icon: "git-pull-request",
  },
];

afterEach(() => setRefKinds([]));

// Every ref opens where its kind's page template says: the ref itself, a
// shell page, an extension's page given the ref — read from the registry,
// core kinds and an extension's alike.
test("a ref opens where its kind's template says, with no row to read", () => {
  setRefKinds(KINDS);
  expect(refTargetNow("work_item:oxplow:tsk1")).toEqual({ tab: workItemTabRef("work_item:oxplow:tsk1") });
  expect(refTargetNow("wiki:home")).toEqual({ tab: wikiPageRef("home") });
  expect(refTargetNow("file:src/a.ts")).toEqual({ tab: fileRef("src/a.ts") });
  expect(refTargetNow("comment:cmt7")).toEqual({ tab: commentsRef() });
  expect(refTargetNow("acme_pr:12")).toEqual({ tab: extPageRef("acme", "pr", { ref: "acme_pr:12" }) });
  expect(refTargetNow("thread:thr4")).toEqual({ thread: "thr4" });
});

// A template that names columns needs its kind's row: the sync path says
// so, the async one reads it (one query on the kind's `resolve` view).
test("a template that reads a column opens through its row", async () => {
  setRefKinds(KINDS);
  expect(refTargetNow("thread_note:not3")).toBeNull();
  const asked: Array<{ view: string; columns: string[]; ref: string }> = [];
  const read = (rows: Record<string, Record<string, unknown>>) => async (view: string, columns: string[], ref: string) => {
    asked.push({ view, columns, ref });
    return rows[ref] ?? null;
  };
  expect(await refTarget("thread_note:not3", read({ "thread_note:not3": { thread_ref: "thread:thr2" } }))).toEqual({
    thread: "thr2",
  });
  expect(asked).toEqual([{ view: "v_thread_note", columns: ["thread_ref"], ref: "thread_note:not3" }]);
  // `{a|b}`: the first that isn't empty.
  expect(
    await refTarget("decision:5", read({ "decision:5": { effort_ref: null, thread_ref: "thread:thr9" } })),
  ).toEqual({ thread: "thr9" });
  // Its row gone: nothing to open.
  expect("unopenable" in (await refTarget("decision:6", read({})))).toBe(true);
});

test("a kind with no page, or one nobody declares, opens nothing", async () => {
  setRefKinds(KINDS);
  expect("unopenable" in (await refTarget("event:00000000-0000-0000-0000-000000000000", async () => null))).toBe(true);
  expect("unopenable" in (await refTarget("gone_kind:1", async () => null))).toBe(true);
});

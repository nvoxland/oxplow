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
  core("comment", "oxplow:oxplow/page?page=comments", "v_comment"),
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
    page: "oxplow:oxplow/page?page=acme/pr&ref=%7Bref%7D",
    icon: "git-pull-request",
  },
];

afterEach(() => setRefKinds([]));

// Every ref opens where its kind's page template says: the ref itself, a
// shell page, an extension's page given the ref — read from the registry,
// core kinds and an extension's alike.
test("a ref opens where its kind's template says, with no row to read", () => {
  setRefKinds(KINDS);
  expect(refTargetNow("oxplow:oxplow/work_item?id=tsk1&provider=oxplow")).toEqual({ tab: workItemTabRef("oxplow:oxplow/work_item?id=tsk1&provider=oxplow") });
  expect(refTargetNow("oxplow:oxplow/wiki?slug=home")).toEqual({ tab: wikiPageRef("home") });
  expect(refTargetNow("oxplow:oxplow/file?path=src/a.ts")).toEqual({ tab: fileRef("src/a.ts") });
  expect(refTargetNow("oxplow:oxplow/comment?id=cmt7")).toEqual({ tab: commentsRef() });
  expect(refTargetNow("acme_pr:12")).toEqual({ tab: extPageRef("acme", "pr", { ref: "acme_pr:12" }) });
  expect(refTargetNow("oxplow:oxplow/thread?id=thr4")).toEqual({ thread: "thr4" });
});

// A template that names columns needs its kind's row: the sync path says
// so, the async one reads it (one query on the kind's `resolve` view).
test("a template that reads a column opens through its row", async () => {
  setRefKinds(KINDS);
  expect(refTargetNow("oxplow:oxplow/thread_note?id=not3")).toBeNull();
  const asked: Array<{ view: string; columns: string[]; ref: string }> = [];
  const read = (rows: Record<string, Record<string, unknown>>) => async (view: string, columns: string[], ref: string) => {
    asked.push({ view, columns, ref });
    return rows[ref] ?? null;
  };
  expect(await refTarget("oxplow:oxplow/thread_note?id=not3", read({ "oxplow:oxplow/thread_note?id=not3": { thread_ref: "oxplow:oxplow/thread?id=thr2" } }))).toEqual({
    thread: "thr2",
  });
  expect(asked).toEqual([{ view: "v_thread_note", columns: ["thread_ref"], ref: "oxplow:oxplow/thread_note?id=not3" }]);
  // `{a|b}`: the first that isn't empty.
  expect(
    await refTarget("oxplow:oxplow/decision?id=5", read({ "oxplow:oxplow/decision?id=5": { effort_ref: null, thread_ref: "oxplow:oxplow/thread?id=thr9" } })),
  ).toEqual({ thread: "thr9" });
  // Its row gone: nothing to open.
  expect("unopenable" in (await refTarget("oxplow:oxplow/decision?id=6", read({})))).toBe(true);
});

test("a kind with no page, or one nobody declares, opens nothing", async () => {
  setRefKinds(KINDS);
  expect("unopenable" in (await refTarget("oxplow:oxplow/event?id=00000000-0000-0000-0000-000000000000", async () => null))).toBe(true);
  expect("unopenable" in (await refTarget("gone_kind:1", async () => null))).toBe(true);
});

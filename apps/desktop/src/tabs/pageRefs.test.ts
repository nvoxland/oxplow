import { describe, expect, test } from "bun:test";
import {
  agentSessionRef,
  goToRef,
  dashboardsRef,
  directoryRef,
  diffRef,
  diffSpecOf,
  settingsRef,
  diskFilePath,
  duplicateBlockRef,
  effortDiffRef,
  endpointDiffRef,
  extPageRef,
  externalUrlRef,
  fileRef,
  gitCommitRef,
  uncommittedChangesRef,
  hookEventsRef,
  pageTabRef,
  metricRef,
  newTaskRef,
  newSessionRef,
  alertsRef,
  refFromTabId,
  fileOf,
  recordedPageKind,
  snapshotRef,
  streamSettingsRef,
  turnRef,
  threadSettingsRef,
  wikiFreshnessRef,
  wikiPageRef,
  workItemTabRef,
  pagePayload,
  resourceParamsOf,
  viewParamsOf,
  withViewParams,
  withResourceParams,
} from "./pageRefs.js";
import type { TabRef } from "./tabState.js";
import { idOf, identityOf, typeOf } from "../uri/uri.js";

describe("pageRefs", () => {
  test("an agent session's tab is its ref, pinned; the picker is a closable page", () => {
    expect(agentSessionRef("ses3")).toEqual({ id: "oxplow:oxplow/agent_session?id=ses3", pinned: true });
    expect(recordedPageKind(agentSessionRef("ses3"))).toBe("oxplow/agent_session");
    expect(newSessionRef()).toEqual({ id: "oxplow:oxplow/page?page=new-session" });
    expect(pagePayload(newSessionRef())).toEqual({ namespace: null, page: "new-session", params: {} });
  });

  test("fileRef encodes the path with a default disk version", () => {
    expect(fileRef("src/a.ts")).toEqual({ id: "oxplow:oxplow/file?path=src/a.ts" });
    expect(fileOf(fileRef("src/a.ts"))).toEqual({ path: "src/a.ts", version: "working" });
  });

  test("a diff is its params: path, both versions, the label and the base's name; its line is its view", () => {
    const spec = { path: "src/a.ts", leftVersion: "git:abc", rightVersion: "working", baseLabel: "abc" };
    const a = diffRef(spec);
    // A base named as its version reads says nothing more: left out. The
    // label the tab says defaults to "diff".
    expect(a.id).toBe("oxplow:oxplow/page?label=diff&left=git:abc&page=diff&path=src/a.ts&right=working");
    expect(pagePayload(a)).toEqual({ namespace: null, page: "diff", params: { path: "src/a.ts", left: "git:abc", right: "working", label: "diff" } });
    expect(diffSpecOf(refFromTabId(a.id)!)).toEqual({ ...spec, labelOverride: undefined, revealLine: undefined });
    const full = { ...spec, baseLabel: "HEAD", labelOverride: "wi 3", revealLine: 7 };
    const b = diffRef(full);
    expect(b.id).toBe("oxplow:oxplow/page?base=HEAD&label=wi%203&left=git:abc&page=diff&path=src/a.ts&right=working#lines=7-7");
    expect(diffSpecOf(refFromTabId(b.id)!)).toEqual(full);
    expect(diffRef({ ...spec, rightVersion: "git:xyz" }).id).not.toBe(a.id);
  });

  test("a Settings area and the section it lands on are its view", () => {
    expect(settingsRef().id).toBe("oxplow:oxplow/page?page=settings");
    expect(settingsRef("overview").id).toBe("oxplow:oxplow/page?page=settings");
    expect(settingsRef("agents", "models").id).toBe("oxplow:oxplow/page?page=settings#area=agents&section=models");
  });

  test("wikiPageRef and workItemTabRef encode canonical refs", () => {
    expect(wikiPageRef("how-x-works").id).toBe("oxplow:oxplow/wiki?slug=how-x-works");
    // A work item's tab is its ref, whichever list (.context/resources.md).
    expect(workItemTabRef("oxplow:oxplow/work_item?id=tsk123&provider=oxplow")).toEqual({
      id: "oxplow:oxplow/work_item?id=tsk123&provider=oxplow",
    });
    expect(gitCommitRef("abc1234")).toEqual({ id: "oxplow:oxplow/commit?sha=abc1234" });
    expect(metricRef("oxplow.todos")).toEqual({ id: "oxplow:oxplow/metric?metric=oxplow.todos" });
  });


  test("oxplow's pages are `oxplow:oxplow/page?page=%3Cid%3E`, any other extension's `oxplow:oxplow/page?page=%3Cext%3E/%3Cid%3E`", () => {
    const ref = pageTabRef("tasks");
    expect(ref.id).toBe("oxplow:oxplow/page?page=tasks");
    expect(typeOf(ref.id)).toBe("oxplow/page");
    expect(hookEventsRef()).toEqual({ id: "oxplow:oxplow/page?page=hook-events" });
    expect(pagePayload(hookEventsRef())).toEqual({ namespace: null, page: "hook-events", params: {} });
    const gh = extPageRef("gh", "open-prs", { uri: "acme_pr:1" });
    expect(gh).toEqual({ id: "oxplow:oxplow/page?page=gh/open-prs&uri=acme_pr:1" });
    expect(pagePayload(gh)).toEqual({ namespace: "gh", page: "open-prs", params: { uri: "acme_pr:1" } });
    expect(uncommittedChangesRef().id).toBe("oxplow:oxplow/page?page=uncommitted-changes");
  });

  test("routes with a subject carry it as query params", () => {
    expect(goToRef().id).toBe("oxplow:oxplow/page?page=go-to");
    expect(wikiFreshnessRef("data-model").id).toBe("oxplow:oxplow/page?page=wiki-freshness&slug=data-model");
    expect(streamSettingsRef("str1").id).toBe("oxplow:oxplow/page?page=stream-settings&stream=str1");
    expect(threadSettingsRef("thr1").id).toBe("oxplow:oxplow/page?page=thread-settings&thread=thr1");
    expect(alertsRef().id).toBe("oxplow:oxplow/page?page=alerts");
  });

  test("route params stay readable: only the ref-reserved and query-syntax characters are escaped", () => {
    // `/` and `:` are legal raw in a ref id and in a query value; `#`, `=`,
    // `&` and `%` are not (a url's own `&` is escaped inside the one `url` value).
    // Intent moved: a web page's tab id is its URL, not a route.
    expect(externalUrlRef("https://x.test/p?a=1&b=2#frag").id).toBe("https://x.test/p?a=1&b=2#frag");
    // Every route id is a valid canonical ref of kind `page`.
    expect(typeOf(diffRef({ path: "a@b/c%d.ts", leftVersion: "working", rightVersion: "working", baseLabel: "" }).id)).toBe("oxplow/page");
  });

  test("a new item's parent is its param", () => {
    expect(newTaskRef().id).toBe("oxplow:oxplow/page?page=new-task");
    expect(newTaskRef({ parentRef: "work_item:tsk3" }).id).toBe("oxplow:oxplow/page?page=new-task&parent=work_item:tsk3");
  });

  test("an effort, snapshot or turn page is its canonical ref (P2.11)", () => {
    const effort = effortDiffRef("eff42");
    expect(effort).toEqual({ id: "oxplow:oxplow/effort?id=eff42" });
    expect(snapshotRef(112)).toEqual({ id: "oxplow:oxplow/snapshot?id=112" });
    expect(turnRef("trn7")).toEqual({ id: "oxplow:oxplow/turn?id=trn7" });
    for (const [ref, kind] of [[effort, "oxplow/effort"], [snapshotRef(112), "oxplow/snapshot"], [turnRef("trn7"), "oxplow/turn"]] as const) {
      expect(refFromTabId(ref.id)).toEqual(ref);
      expect(recordedPageKind(ref)).toBe(kind);
    }
  });

  test("a file path with ref-reserved characters round-trips (P2.11)", () => {
    const path = "docs/a@b#c.md";
    const ref = fileRef(path);
    expect(ref.id).toBe("oxplow:oxplow/file?path=docs/a@b%23c.md");
    expect(diskFilePath(ref.id)).toBe(path);
    expect(refFromTabId(ref.id)).toEqual(ref);
  });

  test("endpointDiffRef encodes both endpoints; ids are stable + distinct", () => {
    const a = endpointDiffRef(
      "snap:1",
      "snap:9",
    );
    const b = endpointDiffRef(
      "snap:1",
      "snap:9",
    );
    expect(a.id).toBe("oxplow:oxplow/page?page=diff-view#end=snap:9&start=snap:1");
    expect(a.id).toBe(b.id);
    const c = endpointDiffRef(null, "git:abc123");
    expect(c.id).toBe("oxplow:oxplow/page?page=diff-view#end=git:abc123&start=none");
    // The endpoints are the page's view: another pair is the same page.
    expect(identityOf(c.id)).toBe(identityOf(a.id));
  });
});

describe("refFromTabId — diff-view", () => {

  test("round-trips snapshot↔snapshot endpoints", () => {
    const ref = endpointDiffRef(
      "snap:1",
      "snap:9",
    );
    expect(refFromTabId(ref.id)).toEqual(ref);
  });

  test("round-trips a null-start commit endpoint and a working endpoint", () => {
    const commitRef = endpointDiffRef(null, "git:abc123");
    expect(refFromTabId(commitRef.id)).toEqual(commitRef);
    const workingRef = endpointDiffRef(
      "snap:5",
      "working",
    );
    expect(refFromTabId(workingRef.id)).toEqual(workingRef);
  });
});

describe("refFromTabId", () => {
  test("rebuilds a file ref with its path payload (the rail-History bug)", () => {
    const r = refFromTabId("oxplow:oxplow/file?path=Cargo.toml")!;
    expect(fileOf(r)?.path).toBe("Cargo.toml");
    expect(r.id).toBe(fileRef("Cargo.toml").id);
  });

  test("handles nested paths and a versioned-viewer revision", () => {
    expect(fileOf(refFromTabId("oxplow:oxplow/file?path=src/a/b.ts")!)?.path).toBe("src/a/b.ts");
    const versioned = refFromTabId("oxplow:oxplow/file?path=src/x.ts&rev=git:abc")!;
    expect(fileOf(versioned)?.path).toBe("src/x.ts");
    expect(fileOf(versioned)?.version).toEqual("git:abc");
  });

  test("rebuilds payload-bearing kinds from their id", () => {
    expect(refFromTabId("oxplow:oxplow/wiki?slug=some-slug")).toEqual(wikiPageRef("some-slug"));
    // `:` inside an id is legal; a naive split on the first colon breaks here.
    expect(refFromTabId("oxplow:oxplow/work_item?id=tsk42&provider=oxplow")).toEqual(workItemTabRef("oxplow:oxplow/work_item?id=tsk42&provider=oxplow"));
    expect(refFromTabId("oxplow:oxplow/commit?sha=abc1234")).toEqual(gitCommitRef("abc1234"));
    expect(refFromTabId("oxplow:oxplow/metric?metric=oxplow.todos")).toEqual(metricRef("oxplow.todos"));
    // Single snapshot is a diff-view ref now (kind "snapshot" is gone).
    expect(refFromTabId(snapshotRef(112).id)).toEqual(snapshotRef(112));
    expect(refFromTabId("https://x.test/p")).toEqual(externalUrlRef("https://x.test/p"));
  });

  test("a page id rebuilds its page and params", () => {
    expect(refFromTabId("oxplow:oxplow/page?page=tasks")).toEqual(pageTabRef("tasks"));
    expect(refFromTabId("oxplow:oxplow/page?area=agents&page=settings")).toEqual(pageTabRef("settings", { area: "agents" }));
    expect(refFromTabId("oxplow:oxplow/page?page=gh/open-prs")).toEqual(extPageRef("gh", "open-prs"));
  });

  test("text that is not a URI, or a scheme oxplow doesn't open, is null (a dead row, not a broken tab)", () => {
    expect(refFromTabId("tasks")).toBeNull();
    expect(refFromTabId("nope:1")).toBeNull();
    expect(refFromTabId("ftp://x.test/a")).toBeNull();
  });

  test("every oxplow URI is a tab, even of a resource nothing declares (the fallback page shows it)", () => {
    // Intent moved: an unknown resource used to be null.
    expect(refFromTabId("oxplow:gone/kind?id=1")).toEqual({ id: "oxplow:gone/kind?id=1" });
  });

  test("a page no extension declares still opens: its page says there's no such page", () => {
    expect(refFromTabId("oxplow:oxplow/page?page=no-such-page")).toEqual(pageTabRef("no-such-page"));
  });

  test("a diff page rebuilds its whole spec from its id", () => {
    const spec = { path: "src/a.ts", leftVersion: "snap:3", rightVersion: "git:HEAD", baseLabel: "x", labelOverride: "eff9", revealLine: undefined } as const;
    const rebuilt = refFromTabId(diffRef(spec).id)!;
    expect(diffSpecOf(rebuilt)).toEqual(spec);
  });

  test("diskFilePath names the working-tree file a tab id shows, or null", () => {
    expect(diskFilePath("oxplow:oxplow/file?path=src/a.ts")).toBe("src/a.ts");
    expect(diskFilePath("oxplow:oxplow/file?path=src/a@b.ts")).toBe("src/a@b.ts");
    // A pinned revision is a read-only viewer, not the editor's file.
    expect(diskFilePath("oxplow:oxplow/file?path=src/a.ts&rev=git:HEAD")).toBeNull();
    // A file on another filesystem (the running extensions) isn't in the
    // working tree, whatever its path; one that names the local one is.
    expect(diskFilePath("oxplow:oxplow/file?path=oxplow-foundation/extension.yaml&filesystem=oxplow:extensions")).toBeNull();
    expect(diskFilePath("oxplow:oxplow/file?path=src/a.ts&filesystem=oxplow:local")).toBe("src/a.ts");
    expect(diskFilePath("oxplow:oxplow/agent_session?id=ses3")).toBeNull();
    expect(diskFilePath("oxplow:oxplow/wiki?slug=a")).toBeNull();
  });

  // tsk163: a ref that can't be rebuilt from its own id is a dead row — the
  // launcher's Recent, the rail's History, and the Go To page all reopen pages
  // by id. Two constructors had drifted: `oxplow:oxplow/dashboard?id=%3Cvariant%3E` had no case at
  // all (so every variant reopened as Planning), and `dir:` was matched under
  // the spelling "directory" (so the case could never fire and the page never
  // opened). Round-trip EVERY constructor rather than spot-checking, so the
  // next one to drift fails here.
  test("every ref rebuilds from its own tab id", () => {
    const refs = [
      agentSessionRef("ses3"),
      newSessionRef(),
      goToRef(),
      dashboardsRef(),
      directoryRef("src/components"),
      externalUrlRef("https://x.test/p"),
      fileRef("src/a.ts"),
      gitCommitRef("abc123"),
      hookEventsRef(),
      pageTabRef("tasks"),
      pageTabRef("git-dashboard"),
      metricRef("oxplow.todos"),
      metricRef("oxplow.todos", { range: "7d", branch: "main" }),
      snapshotRef(112),
      workItemTabRef("oxplow:oxplow/work_item?id=tsk42&provider=oxplow"),
      wikiPageRef("some-slug"),
      effortDiffRef("eff9"),
      pageTabRef("acme/blocked", { stream_id: 2 }),
      pageTabRef("acme/x", { path: "src/a@b.ts", q: "a=b&c" }),
      fileRef("src/a@b.ts"),
      diffRef({ path: "src/a.ts", leftVersion: "working", rightVersion: "git:HEAD", baseLabel: "HEAD" }),
      alertsRef(),
      streamSettingsRef("str1"),
      threadSettingsRef("thr1"),
      wikiFreshnessRef("data-model"),
      uncommittedChangesRef(),
      fileRef("src/a.ts", "git:HEAD"),
      externalUrlRef("https://x.test/p?a=1#frag"),
      duplicateBlockRef({
        leftPath: "a.rs", leftStart: 1, leftEnd: 5, leftVersion: "working",
        rightPath: "b.rs", rightStart: 9, rightEnd: 13, rightVersion: "git:abc",
      }),
    ];
      for (const ref of refs) {
      const rebuilt = refFromTabId(ref.id);
      expect(rebuilt, ref.id).toEqual(ref);
    }
  });

  test("the Go To page reopens as itself", () => {
    expect(refFromTabId(goToRef().id)).toEqual(goToRef());
  });

  test("a directory page reopens with its path payload", () => {
    expect(refFromTabId(directoryRef("src/components").id)).toEqual(
      directoryRef("src/components"),
    );
  });

  test("an extension's page carries its resource params in the `?` and its view in the `#`, and round-trips them", () => {
    const r = pageTabRef("x/effort-tests", { effort_id: 12 }, { label: "a b" });
    expect(r.id).toBe("oxplow:oxplow/page?effort_id=12&page=x/effort-tests#label=a%20b");
    expect(typeOf(r.id)).toBe("oxplow/page");
    expect(resourceParamsOf(r)).toEqual({ effort_id: 12 });
    expect(refFromTabId(r.id)).toEqual(r);
    expect(viewParamsOf(r)).toEqual({ label: "a b" });
  });

  test("a metric's view rides in its URI's `#` and round-trips", () => {
    const r = metricRef("oxplow.todos", { slice: "oxplow.language", range: "2026-06-01..2026-06-03" });
    expect(r.id).toBe("oxplow:oxplow/metric?metric=oxplow.todos#range=2026-06-01..2026-06-03&slice=oxplow.language");
    expect(typeOf(r.id)).toBe("oxplow/metric");
    expect(idOf(r.id, "oxplow/metric")).toBe("oxplow.todos");
    expect(refFromTabId(r.id)).toEqual(r);
    expect(viewParamsOf(r)).toEqual({ slice: "oxplow.language", range: "2026-06-01..2026-06-03" });
    expect(withViewParams(metricRef("oxplow.todos"), { range: "30d" }).id).toBe("oxplow:oxplow/metric?metric=oxplow.todos#range=30d");
    expect(metricRef("oxplow.todos", {}).id).toBe("oxplow:oxplow/metric?metric=oxplow.todos");
  });
});

// Every list's work item opens the one work item page, by its ref.
test("a work item ref opens for every provider", () => {
  const theirs = workItemTabRef("oxplow:oxplow/work_item?id=W-1&provider=fake");
  expect(theirs).toEqual({ id: "oxplow:oxplow/work_item?id=W-1&provider=fake" });
  expect(refFromTabId("oxplow:oxplow/work_item?id=W-1&provider=fake")).toEqual(theirs);
  expect(refFromTabId("oxplow:oxplow/work_item?id=tsk3&provider=oxplow")).toEqual(workItemTabRef("oxplow:oxplow/work_item?id=tsk3&provider=oxplow"));
});



// A page's view is its URI's `#`; the host writes a new view back into the
// same ref, what the page is (its `?`, its payload) unchanged.
describe("view params", () => {
  test("a page's view reads from its URI's `#`", () => {
    expect(viewParamsOf(pageTabRef("x/y", { stream_id: 2 }, { range: "7d", n: 3 }))).toEqual({ range: "7d", n: 3 });
    expect(viewParamsOf(extPageRef("acme", "pulls", {}, { state: "open" }))).toEqual({ state: "open" });
    expect(viewParamsOf(wikiPageRef("home"))).toEqual({});
  });

  test("withViewParams writes the view into the same ref, whatever its kind", () => {
    expect(withViewParams(pageTabRef("x/y", { stream_id: 2 }, { range: "7d" }), { q: "a" }).id).toBe(pageTabRef("x/y", { stream_id: 2 }, { q: "a" }).id);
    expect(withViewParams(extPageRef("acme", "pulls"), { state: "open" }).id).toBe("oxplow:oxplow/page?page=acme/pulls#state=open");
    const own = pageTabRef("settings", {}, { area: "agents", section: "x" });
    expect(withViewParams(own, { area: "data" }).id).toBe("oxplow:oxplow/page?page=settings#area=data");
    expect(withViewParams(wikiPageRef("home"), { lines: "3-3" })).toEqual({ ...wikiPageRef("home"), id: "oxplow:oxplow/wiki?slug=home#lines=3-3" });
  });

  test("withResourceParams names another resource of the same page", () => {
    expect(withResourceParams(pageTabRef("x/y", { stream_id: 1 }, { q: "a" }), { stream_id: 2 }).id).toBe("oxplow:oxplow/page?page=x/y&stream_id=2");
    expect(withResourceParams(pageTabRef("x/y"), { stream_id: 2 }, { q: "b" })).toEqual(pageTabRef("x/y", { stream_id: 2 }, { q: "b" }));
  });
});

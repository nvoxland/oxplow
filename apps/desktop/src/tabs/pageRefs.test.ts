import { describe, expect, test } from "bun:test";
import {
  agentSessionRef,
  customDashboardRef,
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
  endpointsOf,
  extPageRef,
  externalUrlRef,
  fileRef,
  gitCommitRef,
  uncommittedChangesRef,
  hookEventsRef,
  pageTabRef,
  lensRef,
  metricRef,
  newTaskRef,
  newSessionRef,
  alertsRef,
  refFromTabId,
  pageKindOf,
  snapshotRef,
  streamSettingsRef,
  turnRef,
  threadSettingsRef,
  wikiFreshnessRef,
  wikiPageRef,
  workItemTabRef,
  pagePayload,
  viewParamsOf,
  withViewParams,
  type PagePayload,
} from "./pageRefs.js";
import type { TabRef } from "./tabState.js";
import { parseUri } from "../uri/uri.js";

describe("pageRefs", () => {
  test("an agent session's tab is its ref, pinned; the picker is a closable page", () => {
    expect(agentSessionRef("ses3")).toEqual({
      id: "oxplow:oxplow/agent_session?id=ses3",
      kind: "agent_session",
      payload: { sessionId: "ses3" },
      pinned: true,
    });
    expect(pageKindOf("oxplow:oxplow/agent_session?id=ses3")).toBe("agent_session");
    expect(newSessionRef()).toEqual({ id: "oxplow:oxplow/page?page=new-session", kind: "page", payload: { extension: null, page: "new-session", params: {} } });
  });

  test("a dashboard's ref opens its dashboard page", () => {
    expect(refFromTabId("oxplow:oxplow/dashboard?id=dsh3")).toEqual(customDashboardRef("dsh3"));
  });

  test("fileRef encodes the path with a default disk version", () => {
    expect(fileRef("src/a.ts")).toEqual({
      id: "oxplow:oxplow/file?path=src/a.ts",
      kind: "file",
      payload: { path: "src/a.ts", version: "working" },
    });
  });

  test("a diff is its params: path, both versions, the label, the base's name and the line", () => {
    const spec = { path: "src/a.ts", leftVersion: "git:abc", rightVersion: "working", baseLabel: "abc" };
    const a = diffRef(spec);
    // A base named as its version reads says nothing more: left out.
    expect(a.id).toBe("oxplow:oxplow/page?left=git:abc&page=diff&path=src/a.ts&right=working");
    expect(a.payload).toEqual({ extension: null, page: "diff", params: { path: "src/a.ts", left: "git:abc", right: "working" } });
    expect(diffSpecOf((refFromTabId(a.id)!.payload as PagePayload).params)).toEqual({ ...spec, labelOverride: undefined, revealLine: undefined });
    const full = { ...spec, baseLabel: "HEAD", labelOverride: "wi 3", revealLine: 7 };
    const b = diffRef(full);
    expect(b.id).toBe("oxplow:oxplow/page?base=HEAD&label=wi%203&left=git:abc&line=7&page=diff&path=src/a.ts&right=working");
    expect(diffSpecOf((refFromTabId(b.id)!.payload as PagePayload).params)).toEqual(full);
    expect(diffRef({ ...spec, rightVersion: "git:xyz" }).id).not.toBe(a.id);
  });

  test("a Settings area and the section it lands on are its params", () => {
    expect(settingsRef().id).toBe("oxplow:oxplow/page?page=settings");
    expect(settingsRef("overview").id).toBe("oxplow:oxplow/page?page=settings");
    expect(settingsRef("agents", "models").id).toBe("oxplow:oxplow/page?area=agents&page=settings&section=models");
  });

  test("wikiPageRef and workItemTabRef encode canonical refs", () => {
    expect(wikiPageRef("how-x-works").id).toBe("oxplow:oxplow/wiki?slug=how-x-works");
    // A work item's tab is its ref, whichever list (.context/resources.md).
    expect(workItemTabRef("oxplow:oxplow/work_item?id=tsk123&provider=oxplow")).toEqual({
      id: "oxplow:oxplow/work_item?id=tsk123&provider=oxplow",
      kind: "work_item",
      payload: { ref: "oxplow:oxplow/work_item?id=tsk123&provider=oxplow" },
    });
    expect(gitCommitRef("abc1234")).toEqual({ id: "oxplow:oxplow/commit?sha=abc1234", kind: "commit", payload: { sha: "abc1234" } });
    expect(metricRef("oxplow.todos")).toEqual({
      id: "oxplow:oxplow/metric?metric=oxplow.todos",
      kind: "metric",
      payload: { metricKey: "oxplow.todos" },
    });
  });


  test("oxplow's pages are `oxplow:oxplow/page?page=%3Cid%3E`, any other extension's `oxplow:oxplow/page?page=%3Cext%3E/%3Cid%3E`", () => {
    const ref = pageTabRef("tasks");
    expect(ref.id).toBe("oxplow:oxplow/page?page=tasks");
    expect(ref.kind).toBe("page");
    expect(hookEventsRef()).toEqual({ id: "oxplow:oxplow/page?page=hook-events", kind: "page", payload: { extension: null, page: "hook-events", params: {} } });
    expect(extPageRef("gh", "open-prs", { ref: "acme_pr:1" })).toEqual({
      id: "oxplow:oxplow/page?page=gh/open-prs&ref=acme_pr:1",
      kind: "page",
      payload: { extension: "gh", page: "open-prs", params: { ref: "acme_pr:1" } },
    });
    expect(uncommittedChangesRef().id).toBe("oxplow:oxplow/page?page=uncommitted-changes");
  });

  test("routes with a subject carry it as query params", () => {
    expect(goToRef().id).toBe("oxplow:oxplow/page?page=go-to");
    expect(wikiFreshnessRef("data-model").id).toBe("oxplow:oxplow/page?page=wiki-freshness&slug=data-model");
    expect(customDashboardRef("dsh3").id).toBe("oxplow:oxplow/page?id=dsh3&page=custom-dashboard");
    expect(streamSettingsRef("str1").id).toBe("oxplow:oxplow/page?page=stream-settings&stream=str1");
    expect(threadSettingsRef("thr1").id).toBe("oxplow:oxplow/page?page=thread-settings&thread=thr1");
    expect(alertsRef().id).toBe("oxplow:oxplow/page?page=alerts");
  });

  test("route params stay readable: only the ref-reserved and query-syntax characters are escaped", () => {
    // `/` and `:` are legal raw in a ref id and in a query value; `#`, `=`,
    // `&` and `%` are not.
    expect(externalUrlRef("https://x.test/p?a=1&b=2#frag").id)
      .toBe("oxplow:oxplow/page?b=2%23frag&page=external-url&url=https://x.test/p?a%3D1");
    // Every route id is a valid canonical ref of kind `page`.
    expect(parseUri(externalUrlRef("https://x.test/p?a=1#frag").id)?.kind).toBe("page");
    expect(parseUri(diffRef({ path: "a@b/c%d.ts", leftVersion: "working", rightVersion: "working", baseLabel: "" }).id)?.kind).toBe("page");
  });

  test("a new item's parent is its param", () => {
    expect(newTaskRef().id).toBe("oxplow:oxplow/page?page=new-task");
    expect(newTaskRef({ parentRef: "work_item:tsk3" }).id).toBe("oxplow:oxplow/page?page=new-task&parent=work_item:tsk3");
  });

  test("an effort, snapshot or turn page is its canonical ref (P2.11)", () => {
    const effort = effortDiffRef("eff42");
    expect(effort).toEqual({
      id: "oxplow:oxplow/effort?id=eff42",
      kind: "effort",
      payload: { mode: "effort", effortId: "eff42" },
    });
    expect(snapshotRef(112)).toEqual({
      id: "oxplow:oxplow/snapshot?id=112",
      kind: "snapshot",
      payload: { mode: "snapshot", snapshotId: 112 },
    });
    expect(turnRef("trn7")).toEqual({
      id: "oxplow:oxplow/turn?id=trn7",
      kind: "turn",
      payload: { mode: "turn", turnId: "trn7" },
    });
    for (const ref of [effort, snapshotRef(112), turnRef("trn7")]) {
      expect(refFromTabId(ref.id)).toEqual(ref);
      expect(pageKindOf(ref.id)).toBe(ref.kind);
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
    expect(a.id).toBe("oxplow:oxplow/page?end=snap:9&page=diff-view&start=snap:1");
    expect(a.id).toBe(b.id);
    expect(a.kind).toBe("page");
    expect(endpointsOf((a.payload as PagePayload).params)).toEqual({ mode: "endpoints", start: "snap:1", end: "snap:9" });
    const c = endpointDiffRef(null, "git:abc123");
    expect(c.id).toBe("oxplow:oxplow/page?end=git:abc123&page=diff-view&start=none");
    expect(c.id).not.toBe(a.id);
  });
});

describe("refFromTabId — diff-view", () => {
  test("an effort is its own page now: diff-view params without endpoints name none", () => {
    expect(endpointsOf((refFromTabId("oxplow:oxplow/page?effort=eff42&page=diff-view")!.payload as PagePayload).params)).toBeNull();
  });

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
    expect(r.kind).toBe("file");
    expect((r.payload as { path: string }).path).toBe("Cargo.toml");
    expect(r.id).toBe(fileRef("Cargo.toml").id);
  });

  test("handles nested paths and a versioned-viewer revision", () => {
    expect((refFromTabId("oxplow:oxplow/file?path=src/a/b.ts")!.payload as { path: string }).path).toBe("src/a/b.ts");
    const versioned = refFromTabId("oxplow:oxplow/file?path=src/x.ts&rev=git:abc")!;
    expect((versioned.payload as { path: string }).path).toBe("src/x.ts");
    expect((versioned.payload as { version: unknown }).version).toEqual("git:abc");
  });

  test("rebuilds payload-bearing kinds from their id", () => {
    expect(refFromTabId("oxplow:oxplow/wiki?slug=some-slug")).toEqual(wikiPageRef("some-slug"));
    // `:` inside an id is legal; a naive split on the first colon breaks here.
    expect(refFromTabId("oxplow:oxplow/work_item?id=tsk42&provider=oxplow")).toEqual(workItemTabRef("oxplow:oxplow/work_item?id=tsk42&provider=oxplow"));
    expect(refFromTabId("oxplow:oxplow/commit?sha=abc1234")).toEqual(gitCommitRef("abc1234"));
    expect(refFromTabId("oxplow:oxplow/metric?metric=oxplow.todos")).toEqual(metricRef("oxplow.todos"));
    // Single snapshot is a diff-view ref now (kind "snapshot" is gone).
    expect(refFromTabId(snapshotRef(112).id)).toEqual(snapshotRef(112));
    expect(refFromTabId("oxplow:oxplow/page?page=external-url&url=https://x.test/p")).toEqual(externalUrlRef("https://x.test/p"));
  });

  test("a page id rebuilds its page and params", () => {
    expect(refFromTabId("oxplow:oxplow/page?page=tasks")).toEqual(pageTabRef("tasks"));
    expect(refFromTabId("oxplow:oxplow/page?area=agents&page=settings")).toEqual(pageTabRef("settings", { area: "agents" }));
    expect(refFromTabId("oxplow:oxplow/page?page=gh/open-prs")).toEqual(extPageRef("gh", "open-prs"));
  });

  test("text that is not a ref or an unknown kind is null (a dead row, not a broken tab)", () => {
    expect(refFromTabId("tasks")).toBeNull();
    expect(refFromTabId("nope:1")).toBeNull();
  });

  test("a page no extension declares still opens: its page says there's no such page", () => {
    expect(refFromTabId("oxplow:oxplow/page?page=no-such-page")).toEqual(pageTabRef("no-such-page"));
  });

  test("a diff page rebuilds its whole spec from its id", () => {
    const spec = { path: "src/a.ts", leftVersion: "snap:3", rightVersion: "git:HEAD", baseLabel: "x", labelOverride: "eff9", revealLine: undefined } as const;
    const rebuilt = refFromTabId(diffRef(spec).id)!;
    expect(diffSpecOf((rebuilt.payload as PagePayload).params)).toEqual(spec);
  });

  test("diskFilePath names the working-tree file a tab id shows, or null", () => {
    expect(diskFilePath("oxplow:oxplow/file?path=src/a.ts")).toBe("src/a.ts");
    expect(diskFilePath("oxplow:oxplow/file?path=src/a@b.ts")).toBe("src/a@b.ts");
    // A pinned revision is a read-only viewer, not the editor's file.
    expect(diskFilePath("oxplow:oxplow/file?path=src/a.ts&rev=git:HEAD")).toBeNull();
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
      customDashboardRef("dsh1"),
      effortDiffRef("eff9"),
      lensRef("acme/blocked", { stream_id: 2 }),
      lensRef("acme/x", { path: "src/a@b.ts", q: "a=b&c" }),
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
    // A page's look-aside `state` isn't in its id, by design.
    const withoutState = (ref: TabRef): TabRef => {
      if (ref.kind !== "page") return ref;
      const { state: _state, ...payload } = ref.payload as PagePayload;
      return { ...ref, payload };
    };
    for (const ref of refs) {
      const rebuilt = refFromTabId(ref.id);
      expect(rebuilt, ref.id).toEqual(withoutState(ref));
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

  test("lensRef ids the lens and round-trips through refFromTabId", () => {
    const r = lensRef("review/waiting");
    expect(r).toEqual({ id: "oxplow:oxplow/lens?lens=review/waiting", kind: "lens", payload: { lensId: "review/waiting" } });
    expect(refFromTabId(r.id)).toEqual(r);
  });

  test("lensRef carries params in the id, sorted, and round-trips them", () => {
    const r = lensRef("x/effort-tests", { effort_id: 12, label: "a b" });
    expect(r.id).toBe("oxplow:oxplow/lens?effort_id=12&label=a%20b&lens=x/effort-tests");
    expect(parseUri(r.id)?.kind).toBe("lens");
    expect(r.payload).toEqual({ lensId: "x/effort-tests", params: { effort_id: 12, label: "a b" } });
    expect(refFromTabId(r.id)).toEqual(r);
    expect(lensRef("x/y", {}).id).toBe("oxplow:oxplow/lens?lens=x/y");
  });

  test("a metric's view rides in its ref as a query and round-trips", () => {
    const r = metricRef("oxplow.todos", { slice: "oxplow.language", range: "2026-06-01..2026-06-03" });
    expect(r.id).toBe("oxplow:oxplow/metric?metric=oxplow.todos&range=2026-06-01..2026-06-03&slice=oxplow.language");
    expect(parseUri(r.id)).toMatchObject({ kind: "metric", id: "oxplow.todos?range=2026-06-01..2026-06-03&slice=oxplow.language" });
    expect(refFromTabId(r.id)).toEqual(r);
    expect(viewParamsOf(r)).toEqual({ slice: "oxplow.language", range: "2026-06-01..2026-06-03" });
    expect(withViewParams(metricRef("oxplow.todos"), { range: "30d" }).id).toBe("oxplow:oxplow/metric?metric=oxplow.todos&range=30d");
    expect(metricRef("oxplow.todos", {}).id).toBe("oxplow:oxplow/metric?metric=oxplow.todos");
  });
});

// Every list's work item opens the one work item page, by its ref.
test("a work item ref opens for every provider", () => {
  const theirs = workItemTabRef("oxplow:oxplow/work_item?id=W-1&provider=fake");
  expect(theirs).toEqual({ id: "oxplow:oxplow/work_item?id=W-1&provider=fake", kind: "work_item", payload: { ref: "oxplow:oxplow/work_item?id=W-1&provider=fake" } });
  expect(refFromTabId("oxplow:oxplow/work_item?id=W-1&provider=fake")).toEqual(theirs);
  expect(refFromTabId("oxplow:oxplow/work_item?id=tsk3&provider=oxplow")).toEqual(workItemTabRef("oxplow:oxplow/work_item?id=tsk3&provider=oxplow"));
});



// A page's view params are its ref's; the host writes a new view back
// into the same kind of ref, keeping what isn't view (a page's state).
describe("view params", () => {
  test("a lens's and a page's view params read from their ref", () => {
    expect(viewParamsOf(lensRef("x/y", { range: "7d", n: 3 }))).toEqual({ range: "7d", n: 3 });
    expect(viewParamsOf(extPageRef("acme", "pulls", { state: "open" }))).toEqual({ state: "open" });
    expect(viewParamsOf(wikiPageRef("home"))).toEqual({});
  });

  test("withViewParams writes the view into the page's own kind of ref", () => {
    expect(withViewParams(lensRef("x/y", { range: "7d" }), { q: "a" }).id).toBe(lensRef("x/y", { q: "a" }).id);
    expect(withViewParams(extPageRef("acme", "pulls"), { state: "open" }).id).toBe("oxplow:oxplow/page?page=acme/pulls&state=open");
    const own = pageTabRef("settings", { area: "agents", section: "x" });
    expect(withViewParams(own, { area: "data" }).id).toBe("oxplow:oxplow/page?area=data&page=settings");
    // A page with no view keeps its ref.
    const wiki = wikiPageRef("home");
    expect(withViewParams(wiki, { a: "b" })).toBe(wiki);
  });
});

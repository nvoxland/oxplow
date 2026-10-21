import { expect, test } from "bun:test";

import {
  agentSessionUri,
  collectorUri,
  commentUri,
  commitUri,
  configUri,
  contributionOf,
  dirUri,
  effortUri,
  eventUri,
  extPageUri,
  fileUri,
  idOf,
  keyedUri,
  lensUri,
  metricUri,
  pageUri,
  proposalUri,
  streamUri,
  threadUri,
  typeOf,
  wikiUri,
  workItemUri,
} from "./uri.js";

// A command names a record by its URI, never its bare id.

test("each record is named by its URI", () => {
  expect(threadUri("thr3")).toBe("oxplow:oxplow/thread?id=thr3");
  expect(streamUri("str1")).toBe("oxplow:oxplow/stream?id=str1");
  expect(effortUri("eff12")).toBe("oxplow:oxplow/effort?id=eff12");
  expect(commentUri("cmt4")).toBe("oxplow:oxplow/comment?id=cmt4");
  expect(agentSessionUri("ses3")).toBe("oxplow:oxplow/agent_session?id=ses3");
  expect(wikiUri("architecture")).toBe("oxplow:oxplow/wiki?slug=architecture");
  expect(eventUri("0190")).toBe("oxplow:oxplow/event?id=0190");
  expect(proposalUri(7)).toBe("oxplow:oxplow/proposal?id=7");
  expect(configUri("snapshotRetentionDays")).toBe("oxplow:oxplow/config?key=snapshotRetentionDays");
  expect(workItemUri("oxplow", "tsk42")).toBe("oxplow:oxplow/work_item?id=tsk42&provider=oxplow");
  expect(commitUri("4c44d495")).toBe("oxplow:oxplow/commit?sha=4c44d495");
  expect(fileUri("src/a.rs")).toBe("oxplow:oxplow/file?path=src/a.rs");
  expect(fileUri("src/a.rs", "git:HEAD")).toBe("oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD");
  expect(dirUri("src")).toBe("oxplow:oxplow/dir?path=src");
});

test("a contribution's key param is its own name, with the view's params beside it", () => {
  expect(pageUri("tasks")).toBe("oxplow:oxplow/page?page=tasks");
  expect(pageUri("diff-view", { effort: "eff9" })).toBe("oxplow:oxplow/page?effort=eff9&page=diff-view");
  expect(extPageUri("acme", "board", { ref: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" })).toBe(
    "oxplow:oxplow/page?page=acme/board&ref=oxplow:oxplow/work_item?id%3Dtsk1%26provider%3Doxplow",
  );
  expect(lensUri("acme/blocked", { stream_id: 2 })).toBe("oxplow:oxplow/lens?lens=acme/blocked&stream_id=2");
  expect(metricUri("oxplow.todos")).toBe("oxplow:oxplow/metric?metric=oxplow.todos");
  expect(metricUri("oxplow.todos", { range: "7d", branch: "main" })).toBe(
    "oxplow:oxplow/metric?branch=main&metric=oxplow.todos&range=7d",
  );
  expect(collectorUri("triage", "todos")).toBe("oxplow:oxplow/collector?collector=triage/todos");
  expect(contributionOf(lensUri("acme/blocked"))).toEqual({ kind: "lens", extension: "acme", id: "blocked" });
  expect(contributionOf(metricUri("oxplow.todos"))).toEqual({ kind: "metric", extension: null, id: "oxplow.todos" });
  expect(contributionOf(threadUri("thr3"))).toBeNull();
});

test("idOf reads the key param of the one resource", () => {
  expect(idOf(threadUri("thr3"), "oxplow/thread")).toBe("thr3");
  expect(idOf(threadUri("thr3"), "oxplow/stream")).toBeNull();
  expect(idOf("thr3", "oxplow/thread")).toBeNull();
  expect(idOf(wikiUri("a"), "oxplow/wiki")).toBe("a");
  expect(idOf(lensUri("acme/x", { stream_id: 2 }), "oxplow/lens")).toBe("acme/x");
  expect(idOf("oxplow:e2e/item?id=3", "e2e/item")).toBe("3");
  expect(typeOf("oxplow:e2e/item?id=3")).toBe("e2e/item");
  expect(keyedUri("oxplow/wiki", "a")).toBe(wikiUri("a"));
});

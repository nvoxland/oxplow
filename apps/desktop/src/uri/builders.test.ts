import { expect, test } from "bun:test";

import { agentSessionUri, commentUri, configUri, effortUri, eventUri, extPageUri, idOf, lensUri, metricUri, pageUri, proposalUri, streamUri, threadUri, wikiUri, workItemUri } from "./uri.js";

// A command names a record by its ref, never its bare id.

test("each record kind is named by its ref", () => {
  expect(threadUri("thr3")).toBe("thread:thr3");
  expect(streamUri("str1")).toBe("stream:str1");
  expect(effortUri("eff12")).toBe("effort:eff12");
  expect(commentUri("cmt4")).toBe("comment:cmt4");
  expect(agentSessionUri("ses3")).toBe("agent_session:ses3");
  expect(wikiUri("architecture")).toBe("wiki:architecture");
  expect(eventUri("0190")).toBe("event:0190");
  expect(proposalUri(7)).toBe("proposal:7");
  expect(configUri("snapshotRetentionDays")).toBe("config:snapshotRetentionDays");
  expect(workItemUri("oxplow", "tsk42")).toBe("work_item:oxplow:tsk42");
});

test("pages and lenses carry their params as a query", () => {
  expect(pageUri("tasks")).toBe("page:tasks");
  expect(pageUri("diff-view", { effort: "eff9" })).toBe("page:diff-view?effort=eff9");
  expect(extPageUri("acme", "board", { ref: "work_item:oxplow:tsk1" })).toBe(
    "page:acme/board?ref=work_item:oxplow:tsk1",
  );
  expect(lensUri("acme/blocked", { stream_id: 2 })).toBe("lens:acme/blocked?stream_id=2");
  expect(metricUri("oxplow.todos")).toBe("metric:oxplow.todos");
  expect(metricUri("oxplow.todos", { range: "7d", branch: "main" })).toBe("metric:oxplow.todos?range=7d&branch=main");
});

test("idOf reads the id of the one kind", () => {
  expect(idOf("thread:thr3", "thread")).toBe("thr3");
  expect(idOf("thread:thr3", "stream")).toBeNull();
  expect(idOf("thr3", "thread")).toBeNull();
});

import { expect, test } from "bun:test";

import {
  agentSessionRef,
  commentRef,
  configRef,
  effortRef,
  eventRef,
  extPageRef,
  idOf,
  lensRef,
  pageRef,
  proposalRef,
  streamRef,
  threadRef,
  wikiRef,
  workItemRef,
} from "./ref.js";

// A command names a record by its ref, never its bare id.

test("each record kind is named by its ref", () => {
  expect(threadRef("thr3")).toBe("thread:thr3");
  expect(streamRef("str1")).toBe("stream:str1");
  expect(effortRef("eff12")).toBe("effort:eff12");
  expect(commentRef("cmt4")).toBe("comment:cmt4");
  expect(agentSessionRef("ses3")).toBe("agent_session:ses3");
  expect(wikiRef("architecture")).toBe("wiki:architecture");
  expect(eventRef("0190")).toBe("event:0190");
  expect(proposalRef(7)).toBe("proposal:7");
  expect(configRef("snapshotRetentionDays")).toBe("config:snapshotRetentionDays");
  expect(workItemRef("oxplow", "tsk42")).toBe("work_item:oxplow:tsk42");
});

test("pages and lenses carry their params as a query", () => {
  expect(pageRef("tasks")).toBe("page:tasks");
  expect(pageRef("diff-view", { effort: "eff9" })).toBe("page:diff-view?effort=eff9");
  expect(extPageRef("acme", "board", { ref: "work_item:oxplow:tsk1" })).toBe(
    "page:ext.acme.board?ref=work_item:oxplow:tsk1",
  );
  expect(lensRef("acme/blocked", { stream_id: 2 })).toBe("lens:acme/blocked?stream_id=2");
});

test("idOf reads the id of the one kind", () => {
  expect(idOf("thread:thr3", "thread")).toBe("thr3");
  expect(idOf("thread:thr3", "stream")).toBeNull();
  expect(idOf("thr3", "thread")).toBeNull();
});

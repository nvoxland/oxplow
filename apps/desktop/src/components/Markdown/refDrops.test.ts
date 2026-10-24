import { expect, test } from "bun:test";

import { CONTEXT_REF_MIME, WORK_ITEM_DRAG_MIME } from "../../dragMimes.js";
import { fileUri, wikiUri, workItemUri } from "../../uri/uri.js";
import { droppedRefs, pastedLink } from "./refDrops.js";

// What a drop onto the editor names (a context ref, work items), and what
// a paste is: an oxplow URI, a web URL, or neither.

function transfer(data: Record<string, string>) {
  return { types: Object.keys(data), getData: (t: string) => data[t] ?? "" };
}

test("a context ref drop names its URI", () => {
  expect(droppedRefs(transfer({ [CONTEXT_REF_MIME]: JSON.stringify({ kind: "file", path: "src/a.ts" }) }))).toEqual([
    { uri: fileUri("src/a.ts"), title: "src/a.ts" },
  ]);
  expect(droppedRefs(transfer({ [CONTEXT_REF_MIME]: JSON.stringify({ kind: "wiki", slug: "notes" }) }))).toEqual([
    { uri: wikiUri("notes"), title: "notes" },
  ]);
});

test("a work-item drop names each item, with its title", () => {
  const a = workItemUri("oxplow", "tsk1");
  const b = workItemUri("oxplow", "tsk2");
  const drag = { refs: [a, b], items: [{ ref: a, title: "One", state: "ready" }, { ref: b, title: "Two", state: "done" }], fromThreadId: null };
  expect(droppedRefs(transfer({ [WORK_ITEM_DRAG_MIME]: JSON.stringify(drag) }))).toEqual([
    { uri: a, title: "One" },
    { uri: b, title: "Two" },
  ]);
});

test("a foreign drop names nothing", () => {
  expect(droppedRefs(transfer({ "text/plain": "hello" }))).toEqual([]);
});

test("a paste is a URI, a web URL, or plain text", () => {
  expect(pastedLink(` ${wikiUri("notes")}\n`)).toEqual({ kind: "ref", uri: wikiUri("notes") });
  expect(pastedLink("https://example.com/a")).toEqual({ kind: "url", url: "https://example.com/a" });
  expect(pastedLink("see https://example.com")).toBeNull();
  expect(pastedLink("oxplow:not a uri")).toBeNull();
});

import { expect, test } from "bun:test";

import golden from "./openHandlers.golden.json";
import { OPEN_HANDLER_KINDS, openHandlerFor, type OpenHandlers } from "./openHandlers.js";

test("the kinds opened in code only shrink", () => {
  const added = OPEN_HANDLER_KINDS.filter((kind) => !golden.includes(kind));
  expect(added).toEqual([]); // a new kind is a page, not a handler
  const left = golden.filter((kind) => !(OPEN_HANDLER_KINDS as readonly string[]).includes(kind));
  expect(left).toEqual([]); // a kind that left: take it out of the golden too
});

test("a kind with a handler gets it, any other opens as a page tab", () => {
  const seen: string[] = [];
  const handlers = Object.fromEntries(OPEN_HANDLER_KINDS.map((k) => [k, () => seen.push(k)])) as OpenHandlers;
  openHandlerFor(handlers, "oxplow/file")?.({ id: "x" } as never);
  expect(seen).toEqual(["oxplow/file"]);
  expect(openHandlerFor(handlers, "page")).toBeUndefined();
  expect(openHandlerFor(handlers, undefined)).toBeUndefined();
});

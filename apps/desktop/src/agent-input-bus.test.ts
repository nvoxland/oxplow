import { expect, test } from "bun:test";

import { insertIntoAgent, subscribeAgentInput } from "./agent-input-bus.js";

// The terminal pastes what's published here, and xterm turns every line
// break into Enter. Inserting fills the draft and never sends, so the
// bus is the one place line breaks are collapsed — whatever the caller
// (a manifest prompt, a selection, a row) handed it.
test("insertIntoAgent collapses line breaks so a paste can't submit", () => {
  const got: string[] = [];
  const unsub = subscribeAgentInput((text) => got.push(text));
  try {
    insertIntoAgent("Why is the build slow?\nAnd since when?\r\nReally.");
    insertIntoAgent("[oxplow ref file:src/a.ts] what does this do? ");
  } finally {
    unsub();
  }
  expect(got).toEqual([
    "Why is the build slow? And since when? Really.",
    "[oxplow ref file:src/a.ts] what does this do? ",
  ]);
});

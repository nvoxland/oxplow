import { expect, test } from "bun:test";

import { humanize } from "./humanize.js";

// A key with no label reads as words: the first one capitalised.
test("humanize turns a key into a label", () => {
  expect(humanize("committed_at")).toBe("Committed at");
  expect(humanize("subject")).toBe("Subject");
  expect(humanize("effort_id")).toBe("Effort id");
  expect(humanize("parentCount")).toBe("Parent count");
  expect(humanize("q")).toBe("Q");
  expect(humanize("")).toBe("");
});

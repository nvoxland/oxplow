import { describe, expect, test } from "bun:test";
import { fieldDefaults } from "./NewTaskPage.js";

const priority = { name: "priority", title: "Priority", kind: "enum" as const, values: ["high", "low"], read_only: false };
const author = { name: "author", title: "Filed by", kind: "enum" as const, values: ["user", "agent"], read_only: true };
const estimate = { name: "estimate", title: "Estimate", kind: "number" as const, values: [], read_only: false };

describe("fieldDefaults", () => {
  test("a new item starts with no value for any field; the list sets its own", () => {
    expect(fieldDefaults([priority, estimate], {})).toEqual({});
  });

  // Save and Another carries the values just filed forward, so a run of
  // similar items needs them chosen once — only the editable fields the
  // list still declares.
  test("what was last filed carries forward, for the fields the list still declares", () => {
    expect(fieldDefaults([priority, author, estimate], { priority: "high", author: "user", gone: "x", estimate: 3 })).toEqual({
      priority: "high",
      estimate: 3,
    });
  });
});

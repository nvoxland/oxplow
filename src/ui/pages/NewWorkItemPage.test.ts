import { describe, expect, test } from "bun:test";
import { resolveSaveAndAnotherDefaults } from "./NewWorkItemPage.js";

describe("resolveSaveAndAnotherDefaults", () => {
  test("returns the same parent/category/priority that the caller supplied", () => {
    const next = resolveSaveAndAnotherDefaults({
      parentId: "wi-parent",
      initialCategory: "task",
      initialPriority: "high",
      lastCategory: null,
      lastPriority: null,
    });
    expect(next).toEqual({
      parentId: "wi-parent",
      initialCategory: "task",
      initialPriority: "high",
    });
  });

  test("prefers last-saved values over the original initials when both exist", () => {
    const next = resolveSaveAndAnotherDefaults({
      parentId: null,
      initialCategory: "task",
      initialPriority: "medium",
      lastCategory: "bug",
      lastPriority: "urgent",
    });
    expect(next).toEqual({
      parentId: null,
      initialCategory: "bug",
      initialPriority: "urgent",
    });
  });

  test("normalises undefined/empty payload fields to safe defaults", () => {
    const next = resolveSaveAndAnotherDefaults({});
    expect(next).toEqual({
      parentId: null,
      initialCategory: "task",
      initialPriority: "medium",
    });
  });
});

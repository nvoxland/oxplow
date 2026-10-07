import { describe, expect, test } from "bun:test";

import { classifyState } from "../components/Plan/plan-utils.js";
import { CANONICAL_STATES } from "../workItems.js";
import { TASKS_PAGE_SECTIONS } from "./TasksPage.js";

describe("TASKS_PAGE_SECTIONS", () => {
  test("every bucket a state can classify into is rendered — no counted-but-hidden sections", () => {
    for (const state of CANONICAL_STATES) {
      expect(TASKS_PAGE_SECTIONS).toContain(classifyState(state));
    }
  });

  test("in-progress work renders first, above Ready", () => {
    expect(TASKS_PAGE_SECTIONS.indexOf("inProgress")).toBe(0);
    expect(TASKS_PAGE_SECTIONS.indexOf("inProgress")).toBeLessThan(
      TASKS_PAGE_SECTIONS.indexOf("ready"),
    );
  });

  test("an in_progress item classifies into the inProgress section", () => {
    expect(classifyState("in_progress")).toBe("inProgress");
  });
});

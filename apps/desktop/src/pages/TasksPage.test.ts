import { describe, expect, test } from "bun:test";

import { classifyTaskStatus } from "../components/Plan/plan-utils.js";
import { TASKS_PAGE_SECTIONS } from "./TasksPage.js";

describe("TASKS_PAGE_SECTIONS", () => {
  test("every bucket a task status can classify into is rendered — no counted-but-hidden sections", () => {
    const statuses = ["in_progress", "ready", "blocked", "done", "canceled", "archived"] as const;
    for (const status of statuses) {
      const section = classifyTaskStatus(status);
      expect(TASKS_PAGE_SECTIONS).toContain(section);
    }
  });

  test("in-progress work renders first, above Ready", () => {
    expect(TASKS_PAGE_SECTIONS.indexOf("inProgress")).toBe(0);
    expect(TASKS_PAGE_SECTIONS.indexOf("inProgress")).toBeLessThan(
      TASKS_PAGE_SECTIONS.indexOf("ready"),
    );
  });

  test("an in_progress task classifies into the inProgress section", () => {
    expect(classifyTaskStatus("in_progress")).toBe("inProgress");
  });
});

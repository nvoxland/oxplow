import { describe, expect, test } from "bun:test";
import { taskIdOfWorkItemRef, workItemLabel, workItemRef } from "./workItemRef.js";

describe("work item refs", () => {
  test("an oxplow task's ref round-trips to its task id", () => {
    expect(workItemRef("tsk42")).toBe("work_item:oxplow:tsk42");
    expect(taskIdOfWorkItemRef("work_item:oxplow:tsk42")).toBe("tsk42");
  });

  test("another provider's item has no task id", () => {
    expect(taskIdOfWorkItemRef("work_item:issues:ENG-12")).toBeNull();
    expect(taskIdOfWorkItemRef("work_item:oxplow:nope")).toBeNull();
    expect(taskIdOfWorkItemRef("effort:eff1")).toBeNull();
  });

  test("labels name a task by its id and anything else by its provider id", () => {
    expect(workItemLabel("work_item:oxplow:tsk42")).toBe("tsk42");
    expect(workItemLabel("work_item:issues:ENG-12")).toBe("issues:ENG-12");
    expect(workItemLabel("odd")).toBe("odd");
  });
});

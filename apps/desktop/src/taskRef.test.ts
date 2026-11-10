import { describe, expect, test } from "bun:test";
import { taskId, taskLabel } from "./taskRef.js";

describe("task refs", () => {
  test("a ref's provider-scoped id, whichever list", () => {
    expect(taskId("oxplow:oxplow/task?id=tsk42&provider=oxtask")).toBe("oxtask:tsk42");
    expect(taskId("oxplow:oxplow/task?id=ENG-12&provider=issues")).toBe("issues:ENG-12");
    expect(taskId("task:nope")).toBeNull();
    expect(taskId("oxplow:oxplow/request?id=req1")).toBeNull();
  });

  test("a label is the item's own id, as its list gives it", () => {
    expect(taskLabel("oxplow:oxplow/task?id=tsk42&provider=oxtask")).toBe("tsk42");
    expect(taskLabel("oxplow:oxplow/task?id=ENG-12&provider=issues")).toBe("ENG-12");
    expect(taskLabel("odd")).toBe("odd");
  });
});

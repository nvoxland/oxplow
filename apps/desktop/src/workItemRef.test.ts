import { describe, expect, test } from "bun:test";
import { workItemId, workItemLabel } from "./workItemRef.js";

describe("work item refs", () => {
  test("a ref's provider-scoped id, whichever list", () => {
    expect(workItemId("work_item:oxplow:tsk42")).toBe("oxplow:tsk42");
    expect(workItemId("work_item:issues:ENG-12")).toBe("issues:ENG-12");
    expect(workItemId("work_item:nope")).toBeNull();
    expect(workItemId("effort:eff1")).toBeNull();
  });

  test("a label is the item's own id, as its list gives it", () => {
    expect(workItemLabel("work_item:oxplow:tsk42")).toBe("tsk42");
    expect(workItemLabel("work_item:issues:ENG-12")).toBe("ENG-12");
    expect(workItemLabel("odd")).toBe("odd");
  });
});

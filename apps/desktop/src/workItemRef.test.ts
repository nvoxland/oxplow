import { describe, expect, test } from "bun:test";
import { workItemId, workItemLabel } from "./workItemRef.js";

describe("work item refs", () => {
  test("a ref's provider-scoped id, whichever list", () => {
    expect(workItemId("oxplow:oxplow/work_item?id=tsk42&provider=oxplow")).toBe("oxplow:tsk42");
    expect(workItemId("oxplow:oxplow/work_item?id=ENG-12&provider=issues")).toBe("issues:ENG-12");
    expect(workItemId("work_item:nope")).toBeNull();
    expect(workItemId("oxplow:oxplow/effort?id=eff1")).toBeNull();
  });

  test("a label is the item's own id, as its list gives it", () => {
    expect(workItemLabel("oxplow:oxplow/work_item?id=tsk42&provider=oxplow")).toBe("tsk42");
    expect(workItemLabel("oxplow:oxplow/work_item?id=ENG-12&provider=issues")).toBe("ENG-12");
    expect(workItemLabel("odd")).toBe("odd");
  });
});

import { expect, test } from "bun:test";

import type { LensRun } from "../../tauri-bridge/generated/bindings.js";
import { badgeCount } from "./usePanelRuns.js";

// P6.G1: a panel whose badge lens fires shows the alert's count.
test("a firing badge's count; a quiet one has none", () => {
  const run = (firing: boolean, count: number) =>
    ({ alert: { firing, count, value: null, message: "" } }) as unknown as LensRun;
  expect(badgeCount(run(true, 3))).toBe(3);
  expect(badgeCount(run(false, 3))).toBeNull();
  expect(badgeCount(null)).toBeNull();
});

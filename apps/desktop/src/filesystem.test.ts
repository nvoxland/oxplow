import { expect, test } from "bun:test";

import { fsRevLabel, LOCAL_FS } from "./filesystem.js";

test("the local filesystem labels a revision as the revision helpers do", () => {
  expect(fsRevLabel(null, null)).toBe("working tree");
  expect(fsRevLabel(LOCAL_FS, "working")).toBe("working tree");
  expect(fsRevLabel(undefined, "snap:4")).toBe("snapshot 4");
  expect(fsRevLabel(LOCAL_FS, "git:4c44d495fe0a1b2c3d4e")).toBe("4c44d49");
  expect(fsRevLabel(LOCAL_FS, "git:HEAD")).toBe("HEAD");
});

test("another filesystem labels its own revision, or itself", () => {
  expect(fsRevLabel("oxplow:extensions", "4f9c2e1a7b3d5f60")).toBe("4f9c2e1");
  expect(fsRevLabel("oxplow:extensions", "v2")).toBe("v2");
  expect(fsRevLabel("oxplow:temp", null)).toBe("oxplow:temp");
});

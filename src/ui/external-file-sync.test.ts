import { expect, test } from "bun:test";
import { externalFileSyncAction } from "./external-file-sync.js";

const openFile = {
  path: "src/app.ts",
  savedContent: "saved",
  draftContent: "saved",
  isLoading: false,
};

test("externalFileSyncAction ignores unchanged disk content", () => {
  expect(externalFileSyncAction(openFile, "saved")).toBe("noop");
});

test("externalFileSyncAction updates saved content when disk matches the draft", () => {
  expect(externalFileSyncAction({ ...openFile, savedContent: "old", draftContent: "new" }, "new")).toBe("update-saved");
});

test("externalFileSyncAction prompts when disk changed under a dirty draft", () => {
  expect(externalFileSyncAction({ ...openFile, savedContent: "old", draftContent: "mine" }, "theirs")).toBe("prompt");
});

test("externalFileSyncAction replaces a clean draft with disk content", () => {
  expect(externalFileSyncAction(openFile, "theirs")).toBe("replace-draft");
});

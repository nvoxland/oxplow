import { expect, test } from "bun:test";
import {
  closeOpenFile,
  createEmptyFileSession,
  markFileSaved,
  openFileInSession,
  selectOpenFile,
  updateFileDraft,
} from "./file-session.js";

test("openFileInSession tracks multiple open files and keeps ordering stable", () => {
  let state = createEmptyFileSession();
  state = openFileInSession(state, "a.ts", "a");
  state = openFileInSession(state, "b.ts", "b");
  state = openFileInSession(state, "a.ts", "a");

  expect(state.openOrder).toEqual(["a.ts", "b.ts"]);
  expect(state.selectedPath).toBe("a.ts");
});

test("closeOpenFile selects a neighboring tab", () => {
  let state = createEmptyFileSession();
  state = openFileInSession(state, "a.ts", "a");
  state = openFileInSession(state, "b.ts", "b");
  state = openFileInSession(state, "c.ts", "c");
  state = selectOpenFile(state, "b.ts");

  state = closeOpenFile(state, "b.ts");

  expect(state.openOrder).toEqual(["a.ts", "c.ts"]);
  expect(state.selectedPath).toBe("c.ts");
});

test("updateFileDraft and markFileSaved track dirty state per file", () => {
  let state = createEmptyFileSession();
  state = openFileInSession(state, "a.ts", "original");

  state = updateFileDraft(state, "a.ts", "changed");
  expect(state.files["a.ts"]?.draftContent).toBe("changed");
  expect(state.files["a.ts"]?.savedContent).toBe("original");

  state = markFileSaved(state, "a.ts", "changed");
  expect(state.files["a.ts"]?.draftContent).toBe("changed");
  expect(state.files["a.ts"]?.savedContent).toBe("changed");
});

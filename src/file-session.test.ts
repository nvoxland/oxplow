import { expect, test } from "bun:test";
import {
  closeOpenFile,
  createEmptyFileSession,
  markFileSaved,
  openFileInSession,
  removeOpenFiles,
  renameOpenFilePaths,
  selectOpenFile,
  setLoadedFileContent,
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

test("setLoadedFileContent preserves a dirty draft while updating saved content", () => {
  let state = createEmptyFileSession();
  state = openFileInSession(state, "a.ts", "original");
  state = updateFileDraft(state, "a.ts", "mine");

  state = setLoadedFileContent(state, "a.ts", "theirs");

  expect(state.files["a.ts"]?.savedContent).toBe("theirs");
  expect(state.files["a.ts"]?.draftContent).toBe("mine");
});

test("renameOpenFilePaths renames matching open files and preserves selection", () => {
  let state = createEmptyFileSession();
  state = openFileInSession(state, "src/a.ts", "a");
  state = openFileInSession(state, "src/nested/b.ts", "b");
  state = selectOpenFile(state, "src/nested/b.ts");

  state = renameOpenFilePaths(state, (path) => path.startsWith("src/")
    ? `app/${path.slice("src/".length)}`
    : path);

  expect(state.openOrder).toEqual(["app/a.ts", "app/nested/b.ts"]);
  expect(state.selectedPath).toBe("app/nested/b.ts");
  expect(Object.keys(state.files)).toEqual(["app/a.ts", "app/nested/b.ts"]);
});

test("removeOpenFiles closes a batch of paths", () => {
  let state = createEmptyFileSession();
  state = openFileInSession(state, "a.ts", "a");
  state = openFileInSession(state, "b.ts", "b");
  state = openFileInSession(state, "c.ts", "c");

  state = removeOpenFiles(state, ["a.ts", "c.ts"]);

  expect(state.openOrder).toEqual(["b.ts"]);
  expect(state.selectedPath).toBe("b.ts");
});

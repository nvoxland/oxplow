import { expect, test } from "bun:test";
import {
  closeOpenFile,
  createEmptyFileSession,
  enforceOpenFileLimit,
  markFileSaved,
  openFileInSession,
  removeOpenFiles,
  renameOpenFilePaths,
  reorderOpenFiles,
  selectOpenFile,
  setLoadedFileContent,
  updateFileDraft,
} from "./file-session.js";

test("enforceOpenFileLimit closes oldest clean tabs; keeps dirty and selected", () => {
  let state = createEmptyFileSession();
  state = openFileInSession(state, "a.ts", "a");
  state = openFileInSession(state, "b.ts", "b");
  state = openFileInSession(state, "c.ts", "c");
  state = openFileInSession(state, "d.ts", "d");
  state = updateFileDraft(state, "a.ts", "a!"); // a is dirty
  state = selectOpenFile(state, "d.ts");        // d is current

  state = enforceOpenFileLimit(state, 2);

  // Dirty `a.ts` and currently-selected `d.ts` survive; the rest were clean
  // and get closed from the back of the access list.
  expect(state.openOrder.sort()).toEqual(["a.ts", "d.ts"]);
  expect(state.selectedPath).toBe("d.ts");
});

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

test("reorderOpenFiles applies a new openOrder when the path set matches", () => {
  let state = createEmptyFileSession();
  state = openFileInSession(state, "a.ts", "a");
  state = openFileInSession(state, "b.ts", "b");
  state = openFileInSession(state, "c.ts", "c");
  state = selectOpenFile(state, "b.ts");

  state = reorderOpenFiles(state, ["c.ts", "a.ts", "b.ts"]);

  expect(state.openOrder).toEqual(["c.ts", "a.ts", "b.ts"]);
  // Selection and access order are independent of tab layout.
  expect(state.selectedPath).toBe("b.ts");
});

test("reorderOpenFiles is a no-op when the path set does not match", () => {
  let state = createEmptyFileSession();
  state = openFileInSession(state, "a.ts", "a");
  state = openFileInSession(state, "b.ts", "b");

  // Drop a path → reject (length mismatch)
  expect(reorderOpenFiles(state, ["a.ts"]).openOrder).toEqual(["a.ts", "b.ts"]);
  // Foreign path → reject (unknown path)
  expect(reorderOpenFiles(state, ["a.ts", "z.ts"]).openOrder).toEqual(["a.ts", "b.ts"]);
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

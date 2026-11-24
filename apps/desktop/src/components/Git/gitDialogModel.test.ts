import { expect, test } from "bun:test";

import { compareChoices, rollbackRows } from "./gitDialogModel.js";

// Compare with…: one search over the branches, the tags and the file's
// recent commits; what is typed and names none of them is offered as-is.

const data = {
  branches: [
    { name: "main", remote: null },
    { name: "topic", remote: null },
    { name: "main", remote: "origin" },
    { name: "HEAD", remote: "origin" },
  ],
  tags: ["v0.9", "v1.0"],
  commits: [{ id: "abc1234def", short_id: "abc1234", subject: "Fix the main loop", author: "N", time: 0 }],
};

test("with nothing typed: every branch, tag and commit, in that order", () => {
  expect(compareChoices("", data).map((c) => `${c.section}:${c.label}`)).toEqual([
    "Branches:main",
    "Branches:topic",
    "Branches:origin/main",
    "Tags:v0.9",
    "Tags:v1.0",
    "Commits:abc1234 Fix the main loop",
  ]);
});

test("a search keeps what matches it, in a branch, a tag, a sha or a subject", () => {
  expect(compareChoices("main", data).map((c) => c.rev)).toEqual(["main", "origin/main", "abc1234def"]);
  expect(compareChoices("abc12", data).map((c) => c.rev)).toEqual(["abc1234def"]);
});

test("typed text that names none is offered as a revision of its own", () => {
  const choices = compareChoices("HEAD~3", data);
  expect(choices.at(-1)).toEqual({ section: "Revision", label: "Use HEAD~3", rev: "HEAD~3" });
});

// Rollback…: the stream's changes as a checklist, by folder, the target's
// checked; an untracked file says rolling it back deletes it.
test("the changes by folder, the asked ones checked", () => {
  const rows = rollbackRows(
    [
      { path: "src/b.ts", status: "modified" },
      { path: "README.md", status: "modified" },
      { path: "src/a.ts", status: "untracked" },
    ],
    ["src/a.ts"],
  );
  expect(rows).toEqual([
    { folder: "", path: "README.md", name: "README.md", status: "modified", checked: false, deletes: false },
    { folder: "src", path: "src/a.ts", name: "a.ts", status: "untracked", checked: true, deletes: true },
    { folder: "src", path: "src/b.ts", name: "b.ts", status: "modified", checked: false, deletes: false },
  ]);
});

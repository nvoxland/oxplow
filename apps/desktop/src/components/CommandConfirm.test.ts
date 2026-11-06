import { expect, test } from "bun:test";

import { confirmTold } from "./CommandConfirm.js";

const spec = { summary: "Delete it.", confirm: "destructive", confirm_says: null } as const;

test("a command with its own sentence says that, with no warning and no raw input", () => {
  const says = { ...spec, confirm: "always", confirm_says: "Merge {{rev}} into this branch." } as const;
  expect(confirmTold(says, ["Merge main into this branch."])).toEqual({ warn: false, sentence: "Merge main into this branch.", list: [] });
});

test("a destructive command with none keeps its warning, summary and input", () => {
  expect(confirmTold(spec, ["path: a.txt"])).toEqual({ warn: true, sentence: "Delete it.", list: ["path: a.txt"] });
});

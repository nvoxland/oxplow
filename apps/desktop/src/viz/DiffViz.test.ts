import { expect, test } from "bun:test";

import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { withDiffText } from "./DiffViz.js";

const spec: DiffSpec = { path: "a.txt", leftVersion: "working", rightVersion: "working", baseLabel: "working" };

test("a diff page entry that carries literal text (Compare with Clipboard) diffs those sides", () => {
  expect(withDiffText(spec, { diffText: { left: "clip", right: "file" } })).toEqual({ ...spec, leftContent: "clip", rightContent: "file" });
});

test("without that text, the revisions are read", () => {
  expect(withDiffText(spec, undefined)).toBe(spec);
  expect(withDiffText(spec, { diffText: { left: 1 } })).toBe(spec);
});

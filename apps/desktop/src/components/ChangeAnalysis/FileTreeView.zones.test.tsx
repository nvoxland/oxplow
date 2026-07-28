import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

// tsk251: the file tree's zone badge comes from the PROJECT's `zones:`
// table, not a built-in path map. This drives the wiring end to end —
// rules in, badge out — and pins the unconfigured case, where oxplow has
// no vocabulary and the column must stay quiet rather than guess.

import type { BranchChangeEntry } from "../../api.js";
import { ChangeAnalysisFileTree } from "./FileTreeView.js";
import { compileZoneRules } from "./zones.js";
import { __setZoneRulesForTest } from "./useZoneRules.js";

afterEach(() => {
  cleanup();
  __setZoneRulesForTest(null);
});

// Root-level paths so the rows render without expanding a directory.
const FILES: BranchChangeEntry[] = [
  { path: "store.rs", status: "modified", additions: 3, deletions: 1 },
  { path: "deploy.sh", status: "modified", additions: 1, deletions: 0 },
] as unknown as BranchChangeEntry[];

function renderTree() {
  return render(
    <ChangeAnalysisFileTree files={FILES} target="working" onOpenFile={() => {}} />,
  );
}

test("a file badges with the zone its project rule names", () => {
  __setZoneRulesForTest(
    compileZoneRules([{ zone: "store", match: ["*.rs"], color: null }]),
  );
  const { getByText, queryByText } = renderTree();
  expect(getByText("[store]")).toBeDefined();
  // Unmatched files carry no badge — `other` is not a label to show.
  expect(queryByText("[other]")).toBeNull();
});

test("a project with no zone table shows no badges at all", () => {
  __setZoneRulesForTest(compileZoneRules([]));
  const { queryByText } = renderTree();
  expect(queryByText("[store]")).toBeNull();
  expect(queryByText("[other]")).toBeNull();
});

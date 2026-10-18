import { expect, test } from "bun:test";

import { STATUS_COUNTS_SQL, STREAM_HEAD_SQL, STREAM_UPSTREAM_SQL, statusCountsFromRows, streamHeadFromRows, streamUpstreamFromRows } from "./streamHead.js";
import { gitRevision } from "./revision.js";

test("a stream on a branch has its revision and branch", () => {
  expect(streamHeadFromRows([["abc123", "main"]])).toEqual({ revision: gitRevision("abc123"), branch: "main" });
});

test("a detached stream has a revision and no branch; a new one has neither", () => {
  expect(streamHeadFromRows([["abc123", null]])).toEqual({ revision: gitRevision("abc123"), branch: null });
  expect(streamHeadFromRows([[null, null]])).toEqual({ revision: null, branch: null });
  expect(streamHeadFromRows([])).toEqual({ revision: null, branch: null });
});

test("it reads the stream model and the branch it has checked out", () => {
  expect(STREAM_HEAD_SQL).toContain("v_stream");
  expect(STREAM_HEAD_SQL).toContain("v_branch");
});

test("the upstream is the checked-out branch's, with how far they have parted", () => {
  expect(streamUpstreamFromRows([["origin/main", 2, 1]])).toEqual({ upstream: "origin/main", ahead: 2, behind: 1 });
  expect(streamUpstreamFromRows([[null, 0, 0]])).toEqual({ upstream: null, ahead: 0, behind: 0 });
  expect(streamUpstreamFromRows([])).toEqual({ upstream: null, ahead: 0, behind: 0 });
  expect(STREAM_UPSTREAM_SQL).toContain("v_branch");
});

test("uncommitted files are counted by status, untracked apart from added", () => {
  const counts = statusCountsFromRows([
    [1, "added", 0, 2],
    [1, "added", 1, 3],
    [1, "modified", 0, 4],
    [1, "deleted", 0, 1],
  ]);
  expect(counts).toEqual({ added: 2, modified: 4, deleted: 1, renamed: 0, untracked: 3, conflicted: 1, total: 10 });
  // A clean tree answers with one row and no files.
  expect(statusCountsFromRows([[0, null, null, 0]])).toEqual({ added: 0, modified: 0, deleted: 0, renamed: 0, untracked: 0, conflicted: 0, total: 0 });
  expect(statusCountsFromRows([])).toEqual({ added: 0, modified: 0, deleted: 0, renamed: 0, untracked: 0, conflicted: 0, total: 0 });
  expect(STATUS_COUNTS_SQL).toContain("v_change_file");
});

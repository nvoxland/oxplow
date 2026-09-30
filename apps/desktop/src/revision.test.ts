import { expect, test } from "bun:test";
import {
  WORKING,
  gitRevision,
  parseRevision,
  revisionFromSlot,
  revisionSlot,
  shortRevisionLabel,
  snapshotIdOf,
  snapshotRevision,
  targetRevision,
} from "./revision.js";

// P5.B2 (tsk521): a revision is the wire string Rust's `Revision` reads —
// `working`, `snap:<id>`, `<vcs>:<rev>` — with the ref grammar's `@rev`
// slot (working = omitted) as its other spelling.

test("revisions are the wire strings", () => {
  expect(WORKING).toBe("working");
  expect(gitRevision("HEAD")).toBe("git:HEAD");
  expect(snapshotRevision(42)).toBe("snap:42");
  expect(snapshotIdOf("snap:42")).toBe(42);
  expect(snapshotIdOf("git:HEAD")).toBe(null);
});

test("parseRevision accepts only well-formed revisions", () => {
  for (const ok of ["working", "snap:7", "git:abc123", "git:a:b"]) {
    expect(parseRevision(ok)).toBe(ok);
  }
  for (const bad of ["", "HEAD", "snap:x", "git:", ":x", 3, null, { kind: "disk" }]) {
    expect(parseRevision(bad)).toBe(null);
  }
});

test("the rev slot omits the working tree", () => {
  expect(revisionSlot(WORKING)).toBe(null);
  expect(revisionSlot("git:HEAD")).toBe("git:HEAD");
  expect(revisionFromSlot(null)).toBe(WORKING);
  expect(revisionFromSlot("snap:3")).toBe("snap:3");
});

test("short labels", () => {
  expect(shortRevisionLabel(WORKING)).toBe("working tree");
  expect(shortRevisionLabel(gitRevision("0123456789abcdef0123"))).toBe("0123456");
  expect(shortRevisionLabel("git:main")).toBe("main");
  expect(shortRevisionLabel("snap:12")).toBe("snapshot 12");
});

test("a change-analysis target names its revision", () => {
  expect(targetRevision("working")).toBe(WORKING);
  expect(targetRevision("abc1234")).toBe("git:abc1234");
  expect(targetRevision("endpoints:x")).toBe(WORKING);
});

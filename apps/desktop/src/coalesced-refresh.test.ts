import { expect, test } from "bun:test";

// tsk238/tsk240: the branch-changes summary refreshes off BOTH gitRefsChanged
// and workspaceChanged, and each refresh shells out to 4+ git subprocesses
// (including a `status --untracked-files=all` worktree walk). The backend
// watchers already debounce 250ms each, but nothing coalesced *across* the two
// streams and nothing stopped a slow scan from overlapping the next one.

import { coalescedRefresh } from "./coalesced-refresh.js";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** A run that resolves after `ms`, counting starts and completions. */
function tracker(ms = 0) {
  const state = { started: 0, finished: 0 };
  const run = async () => {
    state.started++;
    if (ms) await sleep(ms);
    state.finished++;
  };
  return { state, run };
}

test("a burst of schedules collapses into one run", async () => {
  const { state, run } = tracker();
  const r = coalescedRefresh(run, 20);
  r.schedule();
  r.schedule();
  r.schedule();
  await sleep(60);
  expect(state.started).toBe(1);
});

test("bursts separated by more than the window run separately", async () => {
  const { state, run } = tracker();
  const r = coalescedRefresh(run, 20);
  r.schedule();
  await sleep(60);
  r.schedule();
  await sleep(60);
  expect(state.started).toBe(2);
});

/**
 * The two subscriptions firing near-simultaneously — a `git commit` trips both
 * the gitRefs and workspace debounce windows — must not produce two scans.
 */
test("two streams firing together produce one run", async () => {
  const { state, run } = tracker();
  const r = coalescedRefresh(run, 20);
  r.schedule(); // gitRefsChanged
  await sleep(5);
  r.schedule(); // workspaceChanged
  await sleep(60);
  expect(state.started).toBe(1);
});

test("only one run is in flight at a time", async () => {
  const { state, run } = tracker(60);
  const r = coalescedRefresh(run, 10);
  r.schedule();
  await sleep(30); // run is in flight
  r.schedule();
  await sleep(30); // still in flight — must not have started a second
  expect(state.started).toBe(1);
  expect(state.finished).toBe(0);
});

/** Events arriving mid-run queue exactly one follow-up, not one per event. */
test("schedules during a run queue exactly one follow-up", async () => {
  const { state, run } = tracker(50);
  const r = coalescedRefresh(run, 10);
  r.schedule();
  await sleep(25); // in flight
  r.schedule();
  r.schedule();
  r.schedule();
  await sleep(150);
  expect(state.started).toBe(2);
  expect(state.finished).toBe(2);
});

/** Without a trailing run, the summary would sit stale after a burst. */
test("state changed during a run is picked up by the follow-up", async () => {
  const { state, run } = tracker(40);
  const r = coalescedRefresh(run, 10);
  r.schedule();
  await sleep(25);
  r.schedule();
  await sleep(150);
  expect(state.finished).toBe(2);
});

test("a rejected run does not wedge the gate", async () => {
  let started = 0;
  const r = coalescedRefresh(async () => {
    started++;
    throw new Error("git failed");
  }, 10);
  r.schedule();
  await sleep(40);
  r.schedule();
  await sleep(40);
  expect(started).toBe(2);
});

test("cancel before the window elapses suppresses the run", async () => {
  const { state, run } = tracker();
  const r = coalescedRefresh(run, 30);
  r.schedule();
  r.cancel();
  await sleep(60);
  expect(state.started).toBe(0);
});

/** Teardown during a slow scan must not fire a follow-up into a dead effect. */
test("cancel during a run suppresses the queued follow-up", async () => {
  const { state, run } = tracker(50);
  const r = coalescedRefresh(run, 10);
  r.schedule();
  await sleep(25); // in flight
  r.schedule(); // queues a follow-up
  r.cancel();
  await sleep(150);
  expect(state.started).toBe(1);
});

import { expect, test } from "bun:test";

import { alertCounts, alertRows, alertsSummary, alertKeys, newAlerts, shortAge, type AlertItems, type AlertSources } from "./alertsModel.js";

const none: AlertItems = { proposals: [], opErrors: [], undelivered: 0, failedReactions: 0, extensionProblems: [], badges: [], hints: [] };

// One bell for everything that needs the person.
test("the bell counts what the Alerts page lists, red for a problem, accent for a decision", () => {
  expect(alertsSummary(none)).toEqual({ count: 0, tone: "none" });
  expect(alertsSummary({ ...none, proposals: [{ id: "oxplow:oxplow/proposal?id=1", title: "x" }] })).toEqual({ count: 1, tone: "accent" });
  // A panel's badge that fires is a notice in the Alerts panel, so the icon counts it too.
  expect(alertsSummary({ ...none, badges: [{ id: "x/b", title: "B", message: "2", open: "oxplow:oxplow/page?page=x/b" }] })).toEqual({ count: 1, tone: "accent" });
  expect(
    alertsSummary({
      ...none,
      proposals: [{ id: "oxplow:oxplow/proposal?id=1", title: "x" }],
      opErrors: [{ id: "oe-1", label: "Push" }],
      undelivered: 3,
      failedReactions: 1,
    }),
  ).toEqual({ count: 1 + 1 + 3 + 1, tone: "danger" });
});

// A toast once per new item: a proposal or a failed operation as it
// arrives, delivery as it starts failing, a badge as it starts firing.
test("new alerts are the items not seen before, once each", () => {
  const before = alertKeys({ ...none, proposals: [{ id: "oxplow:oxplow/proposal?id=1", title: "a" }], undelivered: 2 });
  const now: AlertItems = {
    ...none,
    proposals: [
      { id: "oxplow:oxplow/proposal?id=1", title: "a" },
      { id: "oxplow:oxplow/proposal?id=2", title: "Your agent wants to run oxplow.config.set" },
    ],
    opErrors: [{ id: "oe-1", label: "Push failed" }],
    undelivered: 5,
    badges: [{ id: "x/b", title: "Waiting on You", message: "2 items", open: "oxplow:oxplow/page?page=x/b" }],
  };
  expect(newAlerts(before, now).map((a) => a.message)).toEqual([
    "Your agent wants to run oxplow.config.set",
    "Push failed",
    "Waiting on You: 2 items",
  ]);
  // A badge's Review opens its page; the others open Alerts.
  expect(newAlerts(before, now).map((a) => a.open ?? null)).toEqual([null, null, "oxplow:oxplow/page?page=x/b"]);
  expect(newAlerts(alertKeys(now), now)).toEqual([]);
  // Delivery: a toast when it starts failing, not on each new letter.
  expect(newAlerts(alertKeys(none), { ...none, undelivered: 1 }).map((a) => a.message)).toEqual([
    "1 event couldn't be delivered",
  ]);
});

// A hint raised to the person is a notice: it counts, toasts once, and
// isn't a problem.
test("a hint for the person counts and toasts once", () => {
  const hint = { id: 4, kind: "oxplow/landed-in-progress", message: "“t” is still in progress", threadTitle: "Main" };
  expect(alertsSummary({ ...none, hints: [hint] })).toEqual({ count: 1, tone: "accent" });
  expect(newAlerts(alertKeys(none), { ...none, hints: [hint] }).map((a) => a.message)).toEqual(["“t” is still in progress"]);
  expect(newAlerts(alertKeys({ ...none, hints: [hint] }), { ...none, hints: [hint] })).toEqual([]);
});

// An extension held back is a problem: red, counted, toasted once.
test("an extension with problems counts red and toasts once", () => {
  const ext = { name: "oxplow-foundation", message: "`oxplow-foundation` isn't running as it should: boom" };
  expect(alertsSummary({ ...none, extensionProblems: [ext] })).toEqual({ count: 1, tone: "danger" });
  expect(alertKeys({ ...none, extensionProblems: [ext] }).has("ext-problem:oxplow-foundation")).toBe(true);
  expect(newAlerts(alertKeys(none), { ...none, extensionProblems: [ext] }).map((a) => a.message)).toEqual([ext.message]);
  expect(newAlerts(alertKeys({ ...none, extensionProblems: [ext] }), { ...none, extensionProblems: [ext] })).toEqual([]);
});

const noSources: AlertSources = { proposals: [], opErrors: [], undelivered: [], failedReactions: [], extensionProblems: [], badges: [], hints: [] };

test("the panel's rows: decisions, failures, notices, newest first, repeats merged", () => {
  const rows = alertRows({
    ...noSources,
    hints: [{ id: 4, message: "Still in progress", threadTitle: "Main", createdAt: null }],
    opErrors: [
      { id: "a", at: 1_000, label: "Close codex", command: "oxplow.agent_session.close", message: "payload invalid" },
      { id: "b", at: 3_000, label: "Close codex", command: "oxplow.agent_session.close", message: "payload invalid" },
      { id: "c", at: 2_000, label: "New Dashboard…", command: "oxplow.extension.new_dashboard", message: "title is required" },
    ],
    proposals: [{ uri: "oxplow:oxplow/proposal?id=1", createdAt: "1970-01-01T00:00:00.500Z", command: "oxplow.git.push", threadTitle: "T", preview: { summary: "Push main", destructive: false } }],
  });
  expect(rows.map((r) => [r.kind, r.title, r.count])).toEqual([
    ["decision", "“T” wants to run oxplow.git.push", 1],
    ["failure", "Close codex", 2],
    ["failure", "New Dashboard…", 1],
    ["notice", "Still in progress", 1],
  ]);
  expect(rows[1]).toMatchObject({ at: 3_000, detail: "payload invalid", opErrorIds: ["a", "b"] });
  expect(alertCounts(rows)).toEqual({ decision: 1, failure: 3, notice: 1 });
});

// Every alert that has a time says when it happened: the newest of what it
// stands for.
test("each row is at when it happened", () => {
  const rows = alertRows({
    ...noSources,
    undelivered: [{ lastFailedAt: "1970-01-01T00:00:01Z" }, { lastFailedAt: "1970-01-01T00:00:04Z" }],
    failedReactions: [{ failedAt: "1970-01-01T00:00:02Z" }, { failedAt: null }],
    extensionProblems: [
      { name: "a", message: "a broke", at: "1970-01-01T00:00:03Z" },
      { name: "b", message: "b needs approval", at: null },
    ],
    hints: [{ id: 4, message: "Still in progress", threadTitle: null, createdAt: "1970-01-01T00:00:05Z" }],
  });
  expect(rows.map((r) => [r.key, r.at])).toEqual([
    ["delivery", 4_000],
    ["ext-problem:a", 3_000],
    ["reactions", 2_000],
    ["ext-problem:b", null],
    ["hint:4", 5_000],
  ]);
  expect(rows[0]!.title).toBe("2 events couldn't be delivered");
  expect(rows[2]!.title).toBe("2 effects failed to react");
});

test("an age is short", () => {
  const now = 100 * 24 * 3600_000;
  expect(shortAge(now - 10_000, now)).toBe("now");
  expect(shortAge(now - 5 * 60_000, now)).toBe("5m");
  expect(shortAge(now - 3 * 3600_000, now)).toBe("3h");
  expect(shortAge(now - 5 * 24 * 3600_000, now)).toBe("5d");
  expect(shortAge(now - 30 * 24 * 3600_000, now)).toBe("4w");
});

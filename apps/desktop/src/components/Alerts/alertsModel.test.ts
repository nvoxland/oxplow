import { expect, test } from "bun:test";

import { alertsSummary, alertKeys, newAlerts, type AlertItems } from "./alertsModel.js";

const none: AlertItems = { proposals: [], opErrors: [], undelivered: 0, failedReactions: 0, extensionProblems: [], badges: [], hints: [] };

// One bell for everything that needs the person.
test("the bell counts what the Alerts page lists, red for a problem, accent for a decision", () => {
  expect(alertsSummary(none)).toEqual({ count: 0, tone: "none" });
  expect(alertsSummary({ ...none, proposals: [{ id: "oxplow:oxplow/proposal?id=1", title: "x" }] })).toEqual({ count: 1, tone: "accent" });
  // A panel's badge is shown by its panel and toasted; the page doesn't list it, so the bell doesn't count it.
  expect(alertsSummary({ ...none, badges: [{ id: "x/b", title: "B", message: "2", open: "oxplow:oxplow/page?page=x/b" }] })).toEqual({ count: 0, tone: "none" });
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
